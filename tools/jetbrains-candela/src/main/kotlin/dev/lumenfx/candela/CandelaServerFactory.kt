package dev.lumenfx.candela

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.redhat.devtools.lsp4ij.LanguageServerFactory
import com.redhat.devtools.lsp4ij.server.CannotStartProcessException
import com.redhat.devtools.lsp4ij.server.OSProcessStreamConnectionProvider
import com.redhat.devtools.lsp4ij.server.StreamConnectionProvider

/** Connects LSP4IJ to `candela-lsp`, which speaks LSP over stdio and takes no arguments. */
class CandelaServerFactory : LanguageServerFactory {
    override fun createConnectionProvider(project: Project): StreamConnectionProvider = CandelaServer(project)
}

private class CandelaServer(project: Project) : OSProcessStreamConnectionProvider() {

    private val resolved = CandelaLspBinary.resolve(project)

    init {
        val commandLine = GeneralCommandLine(resolved.command)
        project.basePath?.let { commandLine.withWorkDirectory(it) }
        setCommandLine(commandLine)
    }

    override fun start() {
        if (!resolved.found) {
            throw CannotStartProcessException(
                "Cannot find the candela language server. Build it with " +
                    "'${CandelaLspBinary.BUILD_COMMAND}', then put " +
                    "'${CandelaLspBinary.NAME}' on PATH or set its path in " +
                    "Settings | Languages & Frameworks | Candela.",
            )
        }
        super.start()
    }
}
