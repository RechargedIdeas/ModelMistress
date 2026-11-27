package com.modelmistress.core.fold

import com.modelmistress.core.chat.ChatBackend
import com.modelmistress.core.chat.ChatMessage
import com.modelmistress.core.chat.FinishReason
import com.modelmistress.core.chat.GenerationOptions
import com.modelmistress.core.chat.GenerationStats
import com.modelmistress.core.chat.ServedAnswer
import com.modelmistress.core.chat.TokenSink
import com.modelmistress.core.misty.MistyFeedback
import com.modelmistress.core.misty.MistyReflex
import com.modelmistress.core.misty.MistyToolbox
import com.modelmistress.core.misty.MistyTurn
import com.modelmistress.core.tools.ToolCall
import com.modelmistress.core.tools.ToolListener
import com.modelmistress.core.tools.ToolPolicies
import com.modelmistress.core.tools.ToolSource
import com.modelmistress.core.tools.Toolbox

/**
 * The tool loop for a reply the phone's HTTP server writes. There is no chat sandbox here: a request from another app
 * must not be handed the file tools. REA is not offered. The phone cannot start that child.
 */
object ServedFold {
    fun tools(
        deviceStatus: () -> String,
        installedModels: () -> String,
        artexUrl: String?,
        artexToken: String?,
    ): ToolSource = FoldToolbox.around(
        MistyToolbox(Toolbox(workspace = null), deviceStatus, installedModels),
        artexUrl,
        artexToken,
    )

    fun answer(
        chat: ChatBackend,
        messages: List<ChatMessage>,
        options: GenerationOptions,
        tools: ToolSource,
        policies: ToolPolicies,
        confirm: (ToolCall) -> Boolean,
        keepGoing: () -> Boolean,
        feedback: MistyFeedback? = null,
        reflex: MistyReflex? = null,
        notes: (String) -> Unit = {},
    ): ServedAnswer {
        val counted = CountingBackend(chat)
        val turn = MistyTurn(
            chat = counted,
            tools = tools,
            policies = policies,
            reflex = reflex,
            feedback = feedback,
            persona = true,
            notes = notes,
        )
        val result = turn.run(messages, options, keepGoing, confirm, object : ToolListener {})
        return ServedAnswer(
            text = result.answer,
            stats = GenerationStats(counted.promptTokens, counted.completionTokens, counted.finish),
        )
    }

    private class CountingBackend(private val base: ChatBackend) : ChatBackend {
        var promptTokens: Int = 0
        var completionTokens: Int = 0
        var finish: FinishReason = FinishReason.STOP

        override fun complete(messages: List<ChatMessage>, options: GenerationOptions, sink: TokenSink): GenerationStats {
            val stats = base.complete(messages, options, sink)
            promptTokens += stats.promptTokens
            completionTokens += stats.completionTokens
            finish = stats.finishReason
            return stats
        }
    }
}
