package com.modelmistress.core.fold

import com.modelmistress.core.tools.ToolCall
import com.modelmistress.core.tools.ToolClass
import com.modelmistress.core.tools.ToolOutcome
import com.modelmistress.core.tools.ToolParameter
import com.modelmistress.core.tools.ToolSource
import com.modelmistress.core.tools.ToolSpec
import java.io.IOException

/**
 * The device's own tools, plus the nine ARTEX tools when the owner has set an engine URL and token. REA is not here.
 * The phone cannot start that child, and a desktop peer key cannot call it.
 *
 * Read-only calls are [ToolClass.READ]. A call that changes a task is [ToolClass.HUB], so it asks while tool use is
 * on. The result is not cut down: a body over the budget is refused whole.
 */
class FoldToolbox(
    private val base: ToolSource,
    private val client: ArtexClient,
) : ToolSource {
    private val artexSpecs: List<ToolSpec> = SPECS

    override val specs: List<ToolSpec> = base.specs + artexSpecs

    override fun spec(name: String): ToolSpec? = specs.firstOrNull { it.name == name }

    override fun run(call: ToolCall): ToolOutcome {
        if (!isArtex(call.name)) return base.run(call)
        val spec = artexSpecs.firstOrNull { it.name == call.name }
            ?: return ToolOutcome.Refused("there is no tool called ${call.name}")
        val missing = spec.parameters.firstOrNull { it.required && call.arguments[it.name].isNullOrBlank() }
        if (missing != null) return ToolOutcome.Refused("missing argument ${missing.name}")
        return try {
            ToolOutcome.Ok(client.dispatch(call.name, call.arguments))
        } catch (error: ArtexException) {
            ToolOutcome.Failed(error.message ?: "the tool failed")
        } catch (error: IOException) {
            ToolOutcome.Failed(error.message ?: "ARTEX could not be reached")
        }
    }

    companion object {
        /** [base] unchanged when the URL or token is missing, not `http://`, or not safe to put in a header. */
        fun around(base: ToolSource, url: String?, token: String?): ToolSource {
            val client = ArtexClient.tryOpen(url, token) ?: return base
            return FoldToolbox(base, client)
        }

        fun isArtex(name: String): Boolean = name.startsWith("artex_")

        val NAMES: List<String> = listOf(
            "artex_status",
            "artex_tasks",
            "artex_task",
            "artex_scope",
            "artex_scope_add",
            "artex_findings",
            "artex_control",
            "artex_task_create",
            "artex_bind_model",
        )

        private val SPECS: List<ToolSpec> = listOf(
            spec("artex_status", ToolClass.READ, "ARTEX readiness and whether an LLM is configured. A bad token is an error. Read only.", emptyList()),
            spec("artex_tasks", ToolClass.READ, "List ARTEX tasks. Read only.", emptyList()),
            spec("artex_task", ToolClass.READ, "One ARTEX task by numeric id. Read only.", listOf(param("id", "Numeric task id.", true))),
            spec("artex_scope", ToolClass.READ, "Scope rows for one task. Read only.", listOf(param("id", "Numeric task id.", true))),
            spec(
                "artex_scope_add",
                ToolClass.HUB,
                "Add one in-scope host to a task the engine could not seed from the goal. kind is root_domain, subdomain, ip, or cidr. A reason is required. This does not turn the target gate off.",
                listOf(
                    param("id", "Numeric task id.", true),
                    param("kind", "root_domain, subdomain, ip, or cidr.", true),
                    param("value", "The host, IP, or CIDR.", true),
                    param("reason", "Why this target is in scope.", true),
                ),
            ),
            spec(
                "artex_findings",
                ToolClass.READ,
                "One page of 20 findings already recorded. Pass task_id to limit them to one task. Read only. Do not invent findings.",
                listOf(param("task_id", "Numeric task id. Omit it for the latest page across tasks.", false)),
            ),
            spec(
                "artex_control",
                ToolClass.HUB,
                "Pause or resume a task. action is pause or resume.",
                listOf(
                    param("id", "Numeric task id.", true),
                    param("action", "pause or resume.", true),
                ),
            ),
            spec(
                "artex_task_create",
                ToolClass.HUB,
                "Create a task. The goal must name a target you are allowed to test and be at least 80 bytes, unless it contains [targetless]. target_gate is forced on. Credentials are refused. The result says when scope is empty.",
                listOf(
                    param("description", "What this task is.", true),
                    param("goal", "The objective, including the target or [targetless].", true),
                    param("name", "Optional short name.", false),
                ),
            ),
            spec(
                "artex_bind_model",
                ToolClass.HUB,
                "Save the named hub as an OpenAI profile, activate it, and when task_id is set pin that profile on the task. api_key is a peer key, not an owner token. Repeating this updates the profile named ModelMistress.",
                listOf(
                    param("hub_url", "The ModelMistress hub ARTEX should call, http:// or https://.", true),
                    param("model", "Model id on that hub.", true),
                    param("api_key", "A peer key for that model. Not the owner token.", true),
                    param("context_k", "The model's window in K, from 1 to 1000.", true),
                    param("task_id", "Optional numeric task id to pin this profile on.", false),
                ),
            ),
        )

        private fun spec(name: String, toolClass: ToolClass, description: String, parameters: List<ToolParameter>) =
            ToolSpec(name, toolClass, description, parameters)

        private fun param(name: String, description: String, required: Boolean) =
            ToolParameter(name, description, required)
    }
}
