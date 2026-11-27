package com.modelmistress.server

import com.modelmistress.core.chat.ChatBackend
import com.modelmistress.core.chat.ChatMessage
import com.modelmistress.core.chat.FinishReason
import com.modelmistress.core.chat.GenerationOptions
import com.modelmistress.core.chat.GenerationStats
import com.modelmistress.core.chat.ModelCatalog
import com.modelmistress.core.chat.ModelInfo
import com.modelmistress.core.chat.ModelRouter
import com.modelmistress.core.chat.ServedAnswer
import com.modelmistress.core.chat.TokenSink
import com.modelmistress.core.json.Json
import java.net.HttpURLConnection
import java.net.URL
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ServedChatTest {
    private val echo = object : ChatBackend {
        override fun complete(
            messages: List<ChatMessage>,
            options: GenerationOptions,
            sink: TokenSink,
        ): GenerationStats {
            sink.onText("Echo: ${messages.last().content}")
            return GenerationStats(1, 1, FinishReason.STOP)
        }
    }

    private val catalog = object : ModelCatalog {
        override fun models(): List<ModelInfo> = listOf(ModelInfo("tiny", 2048, 10L, "test"))
        override fun defaultModelId(): String? = "tiny"
        override fun backend(modelId: String): ChatBackend? = if (modelId == "tiny") echo else null
    }

    @Test
    fun aServedAnswerReplacesThePlainReplyAndDoesNotLeakTheToolText() {
        val server = ModelMistressServer(
            ModelRouter(catalog),
            ServerConfig(port = 0, apiKey = "k"),
            served = ServedChat { _, _, _, _ ->
                ServedAnswer("The engine is ready.", GenerationStats(3, 4, FinishReason.STOP))
            },
        )
        server.use { running ->
            running.start()
            val (status, body) = call(running.port, "POST", "/v1/chat/completions", CHAT)
            assertEquals(200, status)
            val root = Json.parse(body) as Map<*, *>
            val choice = (root["choices"] as List<*>)[0] as Map<*, *>
            assertEquals("The engine is ready.", (choice["message"] as Map<*, *>)["content"])
            assertFalseTool(body)
            val (streamStatus, stream) = call(running.port, "POST", "/v1/chat/completions", STREAM)
            assertEquals(200, streamStatus)
            assertTrue(stream.contains("The engine is ready."))
            assertTrue(stream.trim().lines().last { it.isNotBlank() }.endsWith("[DONE]"))
            assertFalse(stream.contains("[artex_"))
        }
    }

    @Test
    fun aNullServedAnswerKeepsThePlainPath() {
        val server = ModelMistressServer(
            ModelRouter(catalog),
            ServerConfig(port = 0, apiKey = "k"),
            served = ServedChat { _, _, _, _ -> null },
        )
        server.use { running ->
            running.start()
            val (_, body) = call(running.port, "POST", "/v1/chat/completions", CHAT)
            assertTrue(body.contains("Echo: hi"))
        }
    }

    private fun assertFalseTool(body: String) {
        assertFalse(body.contains("artex_status"))
    }

    private fun assertFalse(value: Boolean) = org.junit.Assert.assertFalse(value)

    private fun call(port: Int, method: String, path: String, body: String): Pair<Int, String> {
        val connection = URL("http://127.0.0.1:$port$path").openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 5_000
        connection.readTimeout = 10_000
        connection.setRequestProperty("Authorization", "Bearer k")
        connection.doOutput = true
        connection.setRequestProperty("Content-Type", "application/json")
        connection.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
        val status = connection.responseCode
        val stream = if (status >= 400) connection.errorStream else connection.inputStream
        return status to stream.bufferedReader(Charsets.UTF_8).use { it.readText() }
    }

    private companion object {
        const val CHAT = """{"messages":[{"role":"user","content":"hi"}]}"""
        const val STREAM = """{"stream":true,"messages":[{"role":"user","content":"hi"}]}"""
    }
}
