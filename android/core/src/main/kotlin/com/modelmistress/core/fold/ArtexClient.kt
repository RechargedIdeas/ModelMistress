package com.modelmistress.core.fold

import com.modelmistress.core.json.Json
import com.modelmistress.core.json.JsonException
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL

/** One HTTP reply after redirects, compression, and the size budget have been refused. */
internal data class ArtexRaw(val status: Int, val body: String)

/** The transport [ArtexClient] uses. Tests pass a stand-in. The phone uses HTTP. */
internal fun interface ArtexExchange {
    fun call(method: String, path: String, body: String?): ArtexRaw
}

/**
 * The nine `artex_` calls, against one engine. Same paths as the desktop hub. `/api/health` is not used: it answers
 * ok with no token. A body over [ArtexGate.RESULT_BUDGET] is refused whole.
 */
class ArtexClient internal constructor(private val exchange: ArtexExchange) {
    fun dispatch(name: String, args: Map<String, String>): String = when (name) {
        "artex_status" -> status()
        "artex_tasks" -> write(exchangeOk("GET", "/api/tasks", null))
        "artex_task" -> {
            val id = ArtexGate.numericId(args, "id")
            write(exchangeOk("GET", "/api/tasks/$id", null))
        }
        "artex_scope" -> {
            val id = ArtexGate.numericId(args, "id")
            write(exchangeOk("GET", "/api/tasks/$id/scope", null))
        }
        "artex_scope_add" -> {
            val (id, body) = ArtexGate.scopeAdd(args)
            write(exchangeOk("POST", "/api/tasks/$id/scope", body))
        }
        "artex_findings" -> {
            val path = StringBuilder("/api/exploration/findings?page=1&limit=20")
            if (args.containsKey("task_id")) {
                path.append("&task_id=").append(ArtexGate.numericId(args, "task_id"))
            }
            write(exchangeOk("GET", path.toString(), null))
        }
        "artex_control" -> {
            val id = ArtexGate.numericId(args, "id")
            val action = args["action"].orEmpty()
            if (action != "pause" && action != "resume") {
                throw ArtexException("action must be pause or resume")
            }
            write(exchangeOk("POST", "/api/tasks/$id/control", linkedMapOf("action" to action)))
        }
        "artex_task_create" -> createTask(args)
        "artex_bind_model" -> bindModel(args)
        else -> throw ArtexException("no tool $name")
    }

    /** Readiness plus whether an LLM is configured. A bad token is an error. */
    private fun status(): String {
        val (readyStatus, ready) = exchangeAny("GET", "/api/ready", null)
        val (statsStatus, stats) = exchangeAny("GET", "/api/stats", null)
        if (statsStatus == 401 || statsStatus == 403) throw ArtexException("ARTEX refused the token")
        if (statsStatus !in 200..299) throw ArtexException("ARTEX $statsStatus: ${clip(write(stats))}")
        val readyMap = ready as? Map<*, *>
        val statsMap = stats as? Map<*, *>
        return write(
            linkedMapOf(
                "ready" to (readyStatus == 200 && readyMap?.get("ready") == true),
                "llm_configured" to statsMap?.get("llm_configured"),
                "engine_mode" to statsMap?.get("engine_mode"),
            ),
        )
    }

    /**
     * Update the profile named ModelMistress, then activate it. An optional task id pins that profile and turns
     * rotation off. The text returned to the model does not include the peer key.
     */
    private fun bindModel(args: Map<String, String>): String {
        val body = ArtexGate.bindBody(args).toMutableMap()
        val listed = exchangeOk("GET", "/api/llm/profiles", null) as? Map<*, *>
        val profiles = listed?.get("profiles") as? List<*>
        val existing = profiles?.firstNotNullOfOrNull { row ->
            val profile = row as? Map<*, *> ?: return@firstNotNullOfOrNull null
            if (profile["name"] == "ModelMistress") idText(profile["id"]) else null
        }
        if (existing != null) body["id"] = existing.toLong()
        val saved = exchangeOk("POST", "/api/llm/profiles", body) as? Map<*, *>
        val id = idText(saved?.get("id"))?.toLongOrNull()
            ?: throw ArtexException("ARTEX did not return a profile id")
        exchangeOk("POST", "/api/llm/profiles/active", linkedMapOf("id" to id))
        val out = linkedMapOf<String, Any?>(
            "id" to id,
            "active" to true,
            "format" to "openai",
            "base_url" to body["base_url"],
            "model" to body["model"],
        )
        if (args.containsKey("task_id")) {
            val task = ArtexGate.numericId(args, "task_id")
            val pinned = exchangeOk(
                "PUT",
                "/api/tasks/$task/llm",
                linkedMapOf(
                    "llm_profile_ids" to listOf(id),
                    "active_llm_profile_id" to id,
                    "llm_rotate" to false,
                ),
            ) as? Map<*, *>
            out["task_id"] = task
            out["pinned"] = true
            out["llm_profile_ids"] = pinned?.get("llm_profile_ids") ?: listOf(id)
        }
        return write(out)
    }

    /**
     * The engine answers 201 with a task object. Scope is a second call. An empty list means the goal did not seed a
     * host, and creating again would make a second task.
     */
    private fun createTask(args: Map<String, String>): String {
        val created = exchangeOk("POST", "/api/tasks", ArtexGate.taskCreateBody(args))
        val objectBody = (created as? Map<*, *>)?.entries?.associate { it.key.toString() to it.value }?.toMutableMap()
            ?: return write(created)
        val id = idText(objectBody["id"])
        if (id == null) {
            objectBody["scope_note"] = "created, but the response had no id, so scope was not read"
            return write(objectBody)
        }
        if (id.any { !it.isDigit() }) {
            objectBody["scope_note"] = "created, but the id was not numeric, so scope was not read"
            return write(objectBody)
        }
        try {
            val scope = exchangeOk("GET", "/api/tasks/$id/scope", null) as? Map<*, *>
            val count = (scope?.get("scope") as? List<*>)?.size ?: 0
            objectBody["scope_count"] = count
            objectBody["scope_empty"] = count == 0
            if (count == 0) {
                objectBody["scope_note"] =
                    "scope is empty. The engine did not seed a host from the goal. " +
                    "Call artex_scope_add before expecting a worker to have a target."
            }
        } catch (error: ArtexException) {
            objectBody["scope_empty"] = true
            objectBody["scope_note"] = "scope could not be read: ${error.message}"
        }
        return write(objectBody)
    }

    private fun exchangeOk(method: String, path: String, body: Map<String, Any?>?): Any? {
        val (status, value) = exchangeAny(method, path, body)
        if (status !in 200..299) throw ArtexException("ARTEX $status: ${clip(write(value))}")
        return value
    }

    private fun exchangeAny(method: String, path: String, body: Map<String, Any?>?): Pair<Int, Any?> {
        val raw = try {
            exchange.call(method, path, body?.let { Json.write(it) })
        } catch (error: ArtexException) {
            throw error
        } catch (error: IOException) {
            if (error is SocketTimeoutException) throw ArtexException("ARTEX: timed out")
            throw ArtexException("ARTEX: ${error.message ?: "could not be reached"}")
        }
        if (raw.body.isBlank()) return raw.status to linkedMapOf("ok" to true)
        val value = try {
            Json.parse(raw.body)
        } catch (error: JsonException) {
            throw ArtexException("ARTEX returned non-JSON: ${clip(raw.body)}")
        }
        return raw.status to value
    }

    private fun write(value: Any?): String = Json.write(value)

    private fun clip(text: String): String {
        if (text.length <= 400) return text
        return text.take(400) + "..."
    }

    companion object {
        /** A client, or null when the URL or token is missing or would not be safe to send. Does not connect. */
        fun tryOpen(base: String?, token: String?): ArtexClient? {
            if (base.isNullOrBlank() || token.isNullOrBlank()) return null
            return try {
                open(base, token)
            } catch (error: ArtexException) {
                null
            }
        }

        fun open(base: String, token: String): ArtexClient {
            val parsed = ArtexGate.parseBase(base)
            val trimmed = token.trim()
            ArtexGate.requireToken(trimmed)
            return ArtexClient(HttpArtexExchange(parsed, trimmed))
        }

        private fun idText(value: Any?): String? = when (value) {
            is String -> value.trim().takeIf { it.isNotEmpty() }
            is Long -> value.toString()
            is Int -> value.toString()
            is Double -> if (value % 1.0 == 0.0) value.toLong().toString() else null
            else -> null
        }
    }
}

/** HTTP/1.1 via [HttpURLConnection]. Redirects are not followed. A gzip body is refused. */
private class HttpArtexExchange(private val base: ArtexGate.Base, private val token: String) : ArtexExchange {
    override fun call(method: String, path: String, body: String?): ArtexRaw {
        val host = if (base.host.contains(':')) "[${base.host}]" else base.host
        val connection = URL("http://$host:${base.port}${base.path}$path").openConnection() as HttpURLConnection
        try {
            connection.instanceFollowRedirects = false
            connection.requestMethod = method
            connection.connectTimeout = TIMEOUT_MS
            connection.readTimeout = TIMEOUT_MS
            connection.useCaches = false
            connection.setRequestProperty("Authorization", "Bearer $token")
            connection.setRequestProperty("Accept", "application/json")
            connection.setRequestProperty("Accept-Encoding", "identity")
            connection.setRequestProperty("Connection", "close")
            if (body != null) {
                val bytes = body.toByteArray(Charsets.UTF_8)
                connection.doOutput = true
                connection.setRequestProperty("Content-Type", "application/json")
                connection.setFixedLengthStreamingMode(bytes.size)
                connection.outputStream.use { it.write(bytes) }
            }
            val status = connection.responseCode
            if (status in 300..399) {
                throw ArtexException("ARTEX redirected. Redirects are not followed.")
            }
            return ArtexRaw(status, readBody(connection, status))
        } finally {
            connection.disconnect()
        }
    }

    private fun readBody(connection: HttpURLConnection, status: Int): String {
        val encoding = connection.getHeaderField("Content-Encoding")
        if (encoding != null && encoding.lowercase().contains("gzip")) {
            throw ArtexException("ARTEX response is compressed. The client refuses it rather than guessing.")
        }
        val declared = connection.getHeaderField("Content-Length")?.trim()?.toLongOrNull()
        if (declared != null && declared > ArtexGate.RESULT_BUDGET) {
            throw ArtexException(ArtexGate.overBudget(declared))
        }
        val stream = if (status >= 400) connection.errorStream else connection.inputStream
        if (stream == null) return ""
        stream.use { input ->
            val out = ByteArrayOutputStream()
            val buf = ByteArray(8192)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                if (out.size().toLong() + n > ArtexGate.RESULT_BUDGET) {
                    throw ArtexException(ArtexGate.overBudget(out.size().toLong() + n))
                }
                out.write(buf, 0, n)
            }
            return out.toString(Charsets.UTF_8)
        }
    }

    private companion object {
        const val TIMEOUT_MS = 20_000
    }
}
