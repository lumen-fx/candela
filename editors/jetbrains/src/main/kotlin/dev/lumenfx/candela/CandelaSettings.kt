package dev.lumenfx.candela

import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.openapi.components.service
import com.intellij.openapi.project.Project

/** Where to find `candela-lsp`, stored per project. */
@Service(Service.Level.PROJECT)
@State(name = "CandelaSettings", storages = [Storage("candela.xml")])
class CandelaSettings : PersistentStateComponent<CandelaSettings.State> {

    class State {
        /** Explicit path to the server binary. Empty means discover it. */
        @JvmField
        var serverPath: String = ""

        /** Probe the project's Cargo target directories before falling back to `PATH`. */
        @JvmField
        var autoDiscover: Boolean = true
    }

    private var state = State()

    override fun getState(): State = state

    override fun loadState(state: State) {
        this.state = state
    }

    companion object {
        fun getInstance(project: Project): CandelaSettings = project.service()
    }
}
