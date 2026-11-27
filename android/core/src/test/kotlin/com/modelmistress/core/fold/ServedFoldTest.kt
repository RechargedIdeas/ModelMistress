package com.modelmistress.core.fold

import com.modelmistress.core.chat.ChatBackend
import com.modelmistress.core.chat.ChatMessage
import com.modelmistress.core.chat.FinishReason
import com.modelmistress.core.chat.GenerationOptions
import com.modelmistress.core.chat.GenerationStats
import com.modelmistress.core.chat.Role
import com.modelmistress.core.chat.TokenSink
import com.modelmistress.core.tools.ToolApproval
import com.modelmistress.core.tools.ToolCall
import com.modelmistress.core.tools.ToolModes
import com.modelmistress.core.tools.ToolUse
import com.modelmistress.core.tools.FileWrites
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ServedFoldTest {
    @Test
    fun aServedRequestGetsArtexButNotFilesOrRea() {
        val bare = ServedFold.tools({ "ok" }, { "none" }, null, null)
        val names = bare.specs.map { it.name }
        assertTrue(names.contains("datetime"))
        assertTrue(names.contains("device_status"))
        assertTrue(names.none { it.startsWith("fs_") })
        assertTrue(names.none { it.startsWith("artex_") })
        assertTrue(names.none { it.startsWith("rea_") })

        val folded = ServedFold.tools({ "ok" }, { "none" }, "http://127.0.0.1:9", "token")
        val offered = folded.specs.map { it.name }
        assertEquals(FoldToolbox.NAMES.toSet(), offered.filter { it.startsWith("artex_") }.toSet())
        assertTrue(offered.none { it.startsWith("fs_") || it.startsWith("rea_") })
    }

    @Test
    fun theClientSeesTheAnswerNotTheToolCall() {
        val seen = mutableListOf<String>()
        val backend = scripted { last ->
            seen += last
            if (last.contains("tool_response_start")) "The engine is ready." else "[artex_status()]"
        }
        val client = ArtexClient { _, path, _ ->
            when (path) {
                "/api/ready" -> ArtexRaw(200, """{"ready":true}""")
                "/api/stats" -> ArtexRaw(200, """{"llm_configured":false,"engine_mode":"idle"}""")
                else -> ArtexRaw(404, """{"error":"no"}""")
            }
        }
        val tools = FoldToolbox(ServedFold.tools({ "free" }, { "local" }, null, null), client)
        val answer = ServedFold.answer(
            chat = backend,
            messages = listOf(ChatMessage(Role.USER, "show the artex status")),
            options = GenerationOptions(maxTokens = 32),
            tools = tools,
            policies = ToolModes.policies(ToolUse.ON, FileWrites.OFF),
            confirm = { false },
            keepGoing = { true },
        )
        assertEquals("The engine is ready.", answer.text)
        assertTrue(seen.any { it.contains(""""ready":true""") || it.contains("ready") })
        assertEquals(2, answer.stats.completionTokens)
        assertFalse(answer.text.contains("artex_status"))
    }

    @Test
    fun aChangeTheOwnerRefusesDoesNotCallTheEngine() {
        var asked: ToolCall? = null
        var posted = false
        val backend = scripted { last ->
            if (last.contains("tool_response_start")) "It was not done." else "[artex_task_create(description=\"lab\", goal=\"short\")]"
        }
        val client = ArtexClient { _, _, _ ->
            posted = true
            ArtexRaw(500, """{"error":"should not be called"}""")
        }
        val tools = FoldToolbox(ServedFold.tools({ "" }, { "" }, null, null), client)
        val answer = ServedFold.answer(
            chat = backend,
            messages = listOf(ChatMessage(Role.USER, "create an artex task")),
            options = GenerationOptions(maxTokens = 32),
            tools = tools,
            policies = ToolModes.policies(ToolUse.ON, FileWrites.OFF),
            confirm = { call ->
                asked = call
                false
            },
            keepGoing = { true },
        )
        assertEquals("artex_task_create", asked?.name)
        assertFalse(posted)
        assertEquals("It was not done.", answer.text)
    }

    @Test
    fun approvalWaitsForTheOwnerAndSilenceRefuses() {
        val gate = ToolApproval(waitMs = 50)
        var refused = false
        var failure: Throwable? = null
        val waiting = Thread {
            try {
                refused = !gate.ask(ToolCall("artex_control", mapOf("action" to "pause")))
            } catch (error: Throwable) {
                failure = error
            }
        }
        waiting.start()
        waiting.join(1_000)
        failure?.let { throw it }
        assertFalse(waiting.isAlive)
        assertTrue(refused)

        val allow = ToolApproval(waitMs = 2_000)
        val shown = java.util.concurrent.atomic.AtomicReference("")
        allow.onChange = { pending -> shown.set(pending?.shown().orEmpty()) }
        var allowed = false
        val worker = Thread {
            try {
                allowed = allow.ask(ToolCall("artex_bind_model", mapOf("api_key" to "peer-secret", "model" to "qwen")))
            } catch (error: Throwable) {
                failure = error
            }
        }
        worker.start()
        val deadline = System.currentTimeMillis() + 1_000
        while (shown.get().isEmpty() && System.currentTimeMillis() < deadline) Thread.sleep(10)
        assertTrue(shown.get().contains("hidden"))
        assertFalse(shown.get().contains("peer-secret"))
        allow.answer(true)
        worker.join(1_000)
        failure?.let { throw it }
        assertFalse(worker.isAlive)
        assertTrue(allowed)
        assertEquals(null, allow.pending)
    }

    private fun scripted(reply: (String) -> String): ChatBackend = object : ChatBackend {
        override fun complete(
            messages: List<ChatMessage>,
            options: GenerationOptions,
            sink: TokenSink,
        ): GenerationStats {
            sink.onText(reply(messages.last().content))
            return GenerationStats(messages.size, 1, FinishReason.STOP)
        }
    }
}
