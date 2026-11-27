package com.modelmistress.core.tools

/**
 * A call the owner has to allow before it runs. The request thread blocks in [ask]. The screen calls [answer].
 * Silence until [waitMs] is a refusal, so a headless request cannot sit forever.
 *
 * [onChange] runs on the request thread and must not call [ask] or [answer].
 */
class ToolApproval(private val waitMs: Long = DEFAULT_WAIT_MS) {
    /** One call waiting for the owner. Secret values are not included in [shown]. */
    data class Pending(val name: String, val arguments: Map<String, String>) {
        fun shown(): String {
            if (arguments.isEmpty()) return "No arguments."
            return arguments.entries.joinToString("\n") { (key, value) ->
                if (isSecret(key)) "$key: (${value.length} characters, hidden)" else "$key: $value"
            }
        }
    }

    @Volatile
    var pending: Pending? = null
        private set

    /** Told when [pending] changes. May be null. */
    @Volatile
    var onChange: ((Pending?) -> Unit)? = null

    private val lock = Object()
    private var decision: Boolean? = null

    /** Blocks until [answer] or the wait runs out. True only when the owner allowed this call. */
    fun ask(call: ToolCall): Boolean {
        synchronized(lock) {
            pending = Pending(call.name, call.arguments)
            decision = null
            onChange?.invoke(pending)
            val deadline = System.currentTimeMillis() + waitMs
            while (decision == null) {
                val left = deadline - System.currentTimeMillis()
                if (left <= 0L) break
                lock.wait(left)
            }
            val allowed = decision == true
            pending = null
            decision = null
            onChange?.invoke(null)
            return allowed
        }
    }

    /** Wakes the request thread. Does nothing when nothing is waiting. */
    fun answer(allow: Boolean) {
        synchronized(lock) {
            if (pending == null) return
            decision = allow
            lock.notifyAll()
        }
    }

    companion object {
        const val DEFAULT_WAIT_MS = 180_000L

        private fun isSecret(key: String): Boolean {
            val lower = key.lowercase()
            return lower == "api_key" || lower == "token" || lower.contains("secret") || lower.contains("password")
        }
    }
}
