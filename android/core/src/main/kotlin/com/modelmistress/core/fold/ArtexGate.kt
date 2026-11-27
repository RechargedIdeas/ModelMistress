package com.modelmistress.core.fold

/**
 * The same gates the desktop hub applies before it talks to ARTEX. This is not a copy of the engine. A goal under 80
 * bytes, or one that names no target, is refused here. `[targetless]` is the only opt-out. Credentials are refused.
 * `target_gate` is always on. There is no switch that turns it off.
 */
object ArtexGate {
    const val RESULT_BUDGET = 48_000

    /** A parsed `http://` base. The host has no brackets. [path] is empty or starts with `/` and has no trailing slash. */
    data class Base(val host: String, val port: Int, val path: String)

    fun overBudget(length: Long): String =
        "ARTEX result is $length bytes, over the $RESULT_BUDGET byte budget. Narrow the query. The body was not truncated."

    fun headerSafe(value: String): Boolean =
        value.isNotEmpty() && value.all { it.code in 0x21..0x7e }

    /**
     * `http://` only. TLS is refused, and the caller must not follow a redirect, so the bearer token stays on this host.
     */
    fun parseBase(base: String): Base {
        val trimmed = base.trim().trimEnd('/')
        val rest = trimmed.removePrefix("http://")
        if (rest.length == trimmed.length) {
            throw ArtexException(
                "ARTEX URL must be http://. The client does not speak TLS and does not follow redirects.",
            )
        }
        if (rest.isEmpty()) throw ArtexException("ARTEX URL has no host")
        val slash = rest.indexOf('/')
        val authority = if (slash < 0) rest else rest.substring(0, slash)
        val path = if (slash < 0) "" else rest.substring(slash).trimEnd('/')
        val (host, port) = if (authority.startsWith("[")) {
            val end = authority.indexOf("]:")
            if (end < 0) throw ArtexException("ARTEX URL has a bad IPv6 host")
            val host = authority.substring(1, end)
            val port = authority.substring(end + 2).toIntOrNull()
                ?: throw ArtexException("ARTEX URL has a bad port")
            host to port
        } else {
            val colon = authority.lastIndexOf(':')
            if (colon >= 0) {
                val host = authority.substring(0, colon)
                if (host.isEmpty()) throw ArtexException("ARTEX URL has no host")
                val port = authority.substring(colon + 1).toIntOrNull()
                    ?: throw ArtexException("ARTEX URL has a bad port")
                host to port
            } else {
                authority to 80
            }
        }
        if (port !in 0..65535) throw ArtexException("ARTEX URL has a bad port")
        if (!headerSafe(host)) throw ArtexException("ARTEX URL host cannot go in a header")
        return Base(host, port, path)
    }

    fun requireToken(token: String) {
        if (!headerSafe(token.trim())) {
            throw ArtexException("ARTEX token is empty or contains characters that cannot go in a header")
        }
    }

    /** Length is UTF-8 bytes, matching the engine. */
    fun admitGoal(goal: String) {
        val trimmed = goal.trim()
        if (trimmed.contains("[targetless]")) return
        if (trimmed.toByteArray(Charsets.UTF_8).size < 80) {
            throw ArtexException(
                "goal is too short to carry a testable objective (min 80 bytes). Name the target and " +
                    "what counts as success. Use [targetless] only when the work has no target.",
            )
        }
        if (goalNamesTarget(trimmed)) return
        throw ArtexException(
            "goal names no target. Include the URL, host, IP, or path you are allowed to test, or " +
                "[targetless] for analysis with no target.",
        )
    }

    /** The `/v1` root an OpenAI provider appends `/chat/completions` to. This is the hub ARTEX will call, not ARTEX itself. */
    fun chatBaseUrl(hubUrl: String): String {
        val trimmed = hubUrl.trim().trimEnd('/')
        if (trimmed.isEmpty()) throw ArtexException("hub url is empty")
        if (!trimmed.startsWith("http://") && !trimmed.startsWith("https://")) {
            throw ArtexException("hub url must start with http:// or https://")
        }
        val withoutCall = trimmed.removeSuffix("/chat/completions").trimEnd('/')
        return if (withoutCall.endsWith("/v1")) withoutCall else "$withoutCall/v1"
    }

    fun numericId(args: Map<String, String>, field: String): String {
        val raw = args[field]?.trim() ?: throw ArtexException("$field is required")
        if (raw.isEmpty() || raw.any { !it.isDigit() }) throw ArtexException("$field must be a numeric id")
        return raw
    }

    /** Body for `POST /api/tasks`. Credentials are refused, not forwarded. */
    fun taskCreateBody(args: Map<String, String>): Map<String, Any?> {
        if (args.containsKey("credentials")) {
            throw ArtexException("credentials are not accepted through the hub")
        }
        val goal = args["goal"].orEmpty()
        admitGoal(goal)
        val description = args["description"].orEmpty().trim()
        if (description.isEmpty()) throw ArtexException("description is required")
        return linkedMapOf(
            "name" to args["name"].orEmpty().trim(),
            "description" to description,
            "goal" to goal.trim(),
            "target_gate" to true,
            "seed_first_intent" to false,
            "timeout_seconds" to 0,
        )
    }

    /**
     * Body for `POST /api/llm/profiles`. Insert does not honor `is_default`, so it is not sent. [api_key] is a peer
     * key for the hub ARTEX will call, never this phone's key and never a desktop owner token.
     */
    fun bindBody(args: Map<String, String>): Map<String, Any?> {
        val contextK = args["context_k"]?.trim()?.toLongOrNull()
        if (contextK == null || contextK !in 1..1000) {
            throw ArtexException("context_k is required (the model's window in K, from 1 to 1000)")
        }
        val key = args["api_key"]?.trim()?.takeIf { it.isNotEmpty() }
            ?: throw ArtexException("api_key is required, and it must be a peer key, not the owner token")
        val model = args["model"].orEmpty().trim()
        if (model.isEmpty()) throw ArtexException("model id is empty")
        val base = chatBaseUrl(args["hub_url"].orEmpty())
        return linkedMapOf(
            "name" to "ModelMistress",
            "format" to "openai",
            "base_url" to base,
            "model" to model,
            "api_key" to key,
            "context_window_k" to contextK,
            "streaming" to true,
            "max_tokens_field" to "max_tokens",
        )
    }

    fun scopeAdd(args: Map<String, String>): Pair<String, Map<String, Any?>> {
        val id = numericId(args, "id")
        val kind = args["kind"].orEmpty().trim()
        if (kind != "root_domain" && kind != "subdomain" && kind != "ip" && kind != "cidr") {
            throw ArtexException("kind must be root_domain, subdomain, ip, or cidr")
        }
        val value = args["value"].orEmpty().trim()
        if (value.isEmpty() || value.length > 200 || value.any { it.code < 0x20 || it.code == 0x7f }) {
            throw ArtexException("value must be a host, IP, or CIDR")
        }
        val reason = args["reason"].orEmpty().trim()
        if (reason.isEmpty() || reason.length > 200) throw ArtexException("reason is required")
        return id to linkedMapOf("kind" to kind, "value" to value, "reason" to reason)
    }

    private fun goalNamesTarget(goal: String): Boolean = goal.split(Regex("\\s+")).any { raw ->
        val token = raw.trim { !kept(it) }
        if (token.contains("://")) return@any true
        val lower = token.lowercase()
        if (lower.startsWith("localhost:")) {
            val port = lower.removePrefix("localhost:")
            return@any port.isNotEmpty() && port.all { it.isDigit() }
        }
        val colon = token.indexOf(':')
        val host = if (colon >= 0) token.substring(0, colon) else token
        val port = if (colon >= 0) token.substring(colon + 1) else null
        if (isIpv4(host)) {
            return@any port == null || (port.isNotEmpty() && port.all { it.isDigit() })
        }
        if (token.startsWith("/") && token.count { it == '/' } >= 2) return@any true
        val trimmedHost = host.trimEnd('.')
        val dot = trimmedHost.lastIndexOf('.')
        if (dot <= 0 || dot == trimmedHost.lastIndex) return@any false
        val name = trimmedHost.substring(0, dot)
        val tld = trimmedHost.substring(dot + 1)
        name.length >= 2 &&
            tld.length >= 2 &&
            tld.all { it in 'a'..'z' || it in 'A'..'Z' } &&
            name.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '-' } &&
            (port == null || (port.isNotEmpty() && port.all { it.isDigit() }))
    }

    private fun kept(c: Char): Boolean =
        c in 'a'..'z' || c in 'A'..'Z' || c in '0'..'9' || c == '/' || c == '.' || c == ':' || c == '-'

    private fun isIpv4(host: String): Boolean {
        val parts = host.split('.')
        if (parts.size != 4) return false
        return parts.all { part ->
            part.isNotEmpty() &&
                part.all { it.isDigit() } &&
                !(part.length > 1 && part.startsWith('0')) &&
                (part.toIntOrNull() ?: -1) in 0..255
        }
    }
}

/** A gate or a response the client will not turn into a tool result. */
class ArtexException(message: String) : IllegalArgumentException(message)
