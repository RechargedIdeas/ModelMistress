package com.modelmistress.core.fold

import com.modelmistress.core.json.Json
import com.modelmistress.core.tools.ToolCall
import com.modelmistress.core.tools.ToolClass
import com.modelmistress.core.tools.ToolOutcome
import com.modelmistress.core.tools.Toolbox
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger

class ArtexFoldTest {
    private val goal = "Confirm the health endpoint of http://127.0.0.1:9 answers, and stop. This is the lab host I am allowed to test."

    @Test
    fun aShortGoalIsRefusedBeforeAnyRequestAndCredentialsAreNotForwarded() {
        var called = false
        val client = client { _, _, _ ->
            called = true
            raw(200, "{}")
        }
        val err = runCatching {
            client.dispatch("artex_task_create", mapOf("description" to "look", "goal" to "check the server"))
        }.exceptionOrNull()
        assertTrue(err?.message?.contains("too short") == true)
        assertFalse(called)
        val withSecret = runCatching {
            client.dispatch(
                "artex_task_create",
                mapOf("description" to "lab", "goal" to goal, "credentials" to "nope"),
            )
        }.exceptionOrNull()
        assertTrue(withSecret?.message?.contains("credentials") == true)
        assertFalse(called)
    }

    @Test
    fun aGoalTheEngineAcceptsIsNotRefusedHere() {
        val pad = "x".repeat(70)
        ArtexGate.admitGoal("Check localhost:8787 and stop. $pad")
        ArtexGate.admitGoal("Check 10.0.0.1:9 and stop. $pad")
        ArtexGate.admitGoal("Read /opt/app/config and stop. $pad")
        ArtexGate.admitGoal("[targetless] just read the notes")
        assertTrue(runCatching { ArtexGate.admitGoal("too short") }.isFailure)
        assertTrue(runCatching { ArtexGate.admitGoal("x".repeat(90)) }.exceptionOrNull()?.message?.contains("no target") == true)
    }

    @Test
    fun theUrlIsHttpOnlyAndTheTokenCannotBreakAHeader() {
        assertEquals("::1", ArtexGate.parseBase("http://[::1]:8787").host)
        assertEquals(8787, ArtexGate.parseBase("http://[::1]:8787").port)
        assertNull(ArtexClient.tryOpen("https://127.0.0.1:8787", "token"))
        assertNull(ArtexClient.tryOpen("http://127.0.0.1:8787", "bad\r\nX: 1"))
        assertNull(ArtexClient.tryOpen("http://127.0.0.1:8787", " "))
        assertTrue(ArtexClient.tryOpen("http://127.0.0.1:8787", "engine-token") != null)
    }

    @Test
    fun bindRequiresAPeerKeyAndDoesNotSendIsDefault() {
        val body = ArtexGate.bindBody(
            mapOf(
                "hub_url" to "http://127.0.0.1:9",
                "model" to "qwen",
                "api_key" to "peer",
                "context_k" to "32",
            ),
        )
        assertEquals("openai", body["format"])
        assertEquals("http://127.0.0.1:9/v1", body["base_url"])
        assertFalse(body.containsKey("is_default"))
        assertEquals("peer", body["api_key"])
        val missing = runCatching {
            ArtexGate.bindBody(mapOf("hub_url" to "http://127.0.0.1:9", "model" to "qwen", "context_k" to "32"))
        }.exceptionOrNull()
        assertTrue(missing?.message?.contains("peer key") == true)
    }

    @Test
    fun scopeKindsAreTheFourTheEngineHasAndFindingsUseTaskId() {
        val bad = runCatching {
            ArtexGate.scopeAdd(mapOf("id" to "3", "kind" to "url", "value" to "example.com", "reason" to "lab"))
        }.exceptionOrNull()
        assertTrue(bad?.message?.contains("kind") == true)
        val calls = mutableListOf<String>()
        val client = client { method, path, _ ->
            calls += "$method $path"
            raw(200, """{"items":[],"total":0,"page":1,"page_size":20}""")
        }
        client.dispatch("artex_findings", mapOf("task_id" to "9"))
        client.dispatch("artex_findings", emptyMap())
        assertEquals("/api/exploration/findings?page=1&limit=20&task_id=9", calls[0].substringAfter(" "))
        assertFalse(calls[0].contains("task="))
        assertFalse(calls[1].contains("task_id"))
        assertFalse(calls[1].contains("task="))
    }

    @Test
    fun createPostsTheGateAndReportsAnEmptyScope() {
        val calls = mutableListOf<Pair<String, String?>>()
        val client = client { method, path, body ->
            calls += "$method $path" to body
            if (path == "/api/tasks") raw(201, """{"id":"7","target_gate":true}""")
            else raw(200, """{"scope":[]}""")
        }
        val text = client.dispatch("artex_task_create", mapOf("description" to "lab", "goal" to goal, "name" to "lab"))
        val created = Json.parse(text) as Map<*, *>
        assertEquals("7", created["id"])
        assertEquals(true, created["scope_empty"])
        assertTrue(created["scope_note"].toString().contains("artex_scope_add"))
        val sent = calls[0].second.orEmpty()
        assertTrue(calls[0].first.startsWith("POST /api/tasks"))
        assertTrue(sent.contains("\"target_gate\":true"))
        assertTrue(sent.contains("\"seed_first_intent\":false"))
        assertFalse(sent.contains("credentials"))
        assertEquals("GET /api/tasks/7/scope", calls[1].first)
    }

    @Test
    fun statusUsesReadyAndStatsAndABadTokenIsAnError() {
        val client = client { _, path, _ ->
            when (path) {
                "/api/ready" -> raw(200, """{"ready":true}""")
                else -> raw(200, """{"llm_configured":true,"engine_mode":"idle"}""")
            }
        }
        val status = Json.parse(client.dispatch("artex_status", emptyMap())) as Map<*, *>
        assertEquals(true, status["ready"])
        assertEquals(true, status["llm_configured"])
        assertEquals("idle", status["engine_mode"])
        val refused = client { _, path, _ ->
            if (path == "/api/ready") raw(200, """{"ready":true}""") else raw(401, """{"error":"no"}""")
        }
        val err = runCatching { refused.dispatch("artex_status", emptyMap()) }.exceptionOrNull()
        assertTrue(err?.message?.contains("refused the token") == true)
    }

    @Test
    fun bindUpdatesTheNamedProfileAndDoesNotReturnTheKey() {
        val calls = mutableListOf<Pair<String, String?>>()
        val client = client { method, path, body ->
            calls += "$method $path" to body
            when {
                path == "/api/llm/profiles" && method == "GET" ->
                    raw(200, """{"profiles":[{"id":4,"name":"ModelMistress","api_key":"stored-secret"}]}""")
                path == "/api/llm/profiles" && method == "POST" -> raw(200, """{"id":4}""")
                path == "/api/llm/profiles/active" -> raw(200, """{"ok":true}""")
                path == "/api/tasks/9/llm" ->
                    raw(200, """{"id":"9","llm_profile_ids":[4],"active_llm_profile_id":4,"llm_rotate":false}""")
                else -> raw(404, """{"error":"unmapped"}""")
            }
        }
        val text = client.dispatch(
            "artex_bind_model",
            mapOf(
                "hub_url" to "http://127.0.0.1:9",
                "model" to "qwen",
                "api_key" to "peer-secret",
                "context_k" to "32",
                "task_id" to "9",
            ),
        )
        assertFalse(text.contains("peer-secret"))
        assertFalse(text.contains("stored-secret"))
        val saved = calls.first { it.first == "POST /api/llm/profiles" }.second.orEmpty()
        assertTrue(saved.contains("\"id\":4"))
        assertTrue(saved.contains("\"api_key\":\"peer-secret\""))
        assertFalse(saved.contains("is_default"))
        val pinned = calls.first { it.first.startsWith("PUT ") }.second.orEmpty()
        assertTrue(pinned.contains("\"llm_rotate\":false"))
        val out = Json.parse(text) as Map<*, *>
        assertEquals(true, out["pinned"])
        assertEquals("9", out["task_id"])
    }

    @Test
    fun thePhoneClientRefusesRedirectsCompressionAndAnOversizeBody() {
        val requests = CopyOnWriteArrayList<String>()
        val script = listOf<(String) -> ByteArray>(
            { http(302, "Location: http://127.0.0.1:9/elsewhere\r\nContent-Length: 0\r\n", "") },
            { http(200, "Content-Encoding: gzip\r\nContent-Length: 2\r\n", "xx") },
            {
                val size = ArtexGate.RESULT_BUDGET + 8
                http(200, "Content-Length: $size\r\n", "x".repeat(size))
            },
            { http(200, "Transfer-Encoding: chunked\r\n", "b\r\n{\"ok\":true}\r\n0\r\n\r\n") },
        )
        val base = serve(script, requests)
        val client = ArtexClient.open(base, "engine-token")
        val redirect = runCatching { client.dispatch("artex_tasks", emptyMap()) }.exceptionOrNull()
        assertTrue(redirect?.message?.contains("not followed") == true)
        val gzip = runCatching { client.dispatch("artex_tasks", emptyMap()) }.exceptionOrNull()
        assertTrue(gzip?.message?.contains("compressed") == true)
        val huge = runCatching { client.dispatch("artex_tasks", emptyMap()) }.exceptionOrNull()
        assertTrue(huge?.message?.contains("not truncated") == true)
        val chunked = client.dispatch("artex_tasks", emptyMap())
        assertEquals(true, (Json.parse(chunked) as Map<*, *>)["ok"])
        assertEquals(4, requests.size)
        assertTrue(requests.all { it.contains("Authorization: Bearer engine-token") })
        assertTrue(requests.none { it.contains("http://127.0.0.1:9/elsewhere") })
    }

    @Test
    fun foldedToolsAreOfferedOnlyWhenTheLinkIsSafeAndReaIsNeverOffered() {
        val plain = Toolbox()
        assertTrue(FoldToolbox.around(plain, null, null).specs.none { it.name.startsWith("artex_") })
        assertTrue(FoldToolbox.around(plain, "https://127.0.0.1:1", "token").specs.none { it.name.startsWith("artex_") })
        val folded = FoldToolbox.around(plain, "http://127.0.0.1:1", "token")
        assertEquals(FoldToolbox.NAMES.toSet(), folded.specs.map { it.name }.filter { it.startsWith("artex_") }.toSet())
        assertEquals(ToolClass.READ, folded.spec("artex_status")?.toolClass)
        assertEquals(ToolClass.READ, folded.spec("artex_findings")?.toolClass)
        assertEquals(ToolClass.HUB, folded.spec("artex_task_create")?.toolClass)
        assertEquals(ToolClass.HUB, folded.spec("artex_scope_add")?.toolClass)
        assertEquals(ToolClass.HUB, folded.spec("artex_bind_model")?.toolClass)
        assertTrue(folded.specs.none { it.name.startsWith("rea_") })
    }

    @Test
    fun aFoldedCallUsesTheClientAndDoesNotTruncateALegalResult() {
        val client = client { _, _, _ -> raw(200, """{"ready":true}""") }
        val tools = FoldToolbox(Toolbox(), client)
        val outcome = tools.run(ToolCall("artex_tasks", emptyMap()))
        assertTrue(outcome is ToolOutcome.Ok)
        assertEquals("""{"ready":true}""", (outcome as ToolOutcome.Ok).text)
        val refused = tools.run(ToolCall("artex_task_create", mapOf("description" to "lab", "goal" to "short")))
        assertTrue(refused is ToolOutcome.Failed)
        assertTrue((refused as ToolOutcome.Failed).reason.contains("too short"))
    }

    private fun client(handler: (String, String, String?) -> ArtexRaw) =
        ArtexClient { method, path, body -> handler(method, path, body) }

    private fun raw(status: Int, body: String) = ArtexRaw(status, body)

    private fun http(status: Int, headers: String, body: String): ByteArray {
        val head = "HTTP/1.1 $status OK\r\n${headers}Connection: close\r\n\r\n"
        return head.toByteArray() + if (headers.contains("Transfer-Encoding")) body.toByteArray() else body.toByteArray()
    }

    /** Serves [handlers] in order on 127.0.0.1 and returns the base URL. */
    private fun serve(handlers: List<(String) -> ByteArray>, seen: MutableList<String>): String {
        val server = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val next = AtomicInteger(0)
        Thread {
            try {
                repeat(handlers.size) {
                    val socket = server.accept()
                    socket.soTimeout = 3_000
                    try {
                        val request = readRequest(socket)
                        seen += request
                        val index = next.getAndIncrement()
                        try {
                            socket.getOutputStream().write(handlers[index](request))
                        } catch (error: Exception) {
                            // The client may close once it has refused a body it will not read.
                        }
                    } finally {
                        socket.close()
                    }
                }
            } finally {
                server.close()
            }
        }.apply { isDaemon = true }.start()
        return "http://127.0.0.1:${server.localPort}"
    }

    private fun readRequest(socket: Socket): String {
        val input = socket.getInputStream()
        val raw = ByteArrayOutputLike()
        val buf = ByteArray(4096)
        while (true) {
            val n = input.read(buf)
            if (n < 0) break
            raw.write(buf, n)
            val text = raw.text()
            val headerEnd = text.indexOf("\r\n\r\n")
            if (headerEnd >= 0) {
                val headers = text.substring(0, headerEnd)
                val length = headers.lineSequence().firstNotNullOfOrNull { line ->
                    val parts = line.split(":", limit = 2)
                    if (parts.size == 2 && parts[0].equals("Content-Length", true)) parts[1].trim().toIntOrNull() else null
                } ?: 0
                if (text.length >= headerEnd + 4 + length) return text
            }
            if (raw.size > 64_000) return raw.text()
        }
        return raw.text()
    }
}

/** A small buffer so the test server can see the request without pulling in extra libraries. */
private class ByteArrayOutputLike {
    private val out = java.io.ByteArrayOutputStream()
    val size: Int get() = out.size()
    fun write(buf: ByteArray, n: Int) = out.write(buf, 0, n)
    fun text(): String = out.toString(Charsets.UTF_8)
}
