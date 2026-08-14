package dev.lumenfx.candela

import com.intellij.execution.configurations.PathEnvironmentVariableUtil
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.ProjectRootManager
import com.intellij.openapi.util.SystemInfo
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths

/**
 * Finds the `candela-lsp` binary the same way the VS Code extension does: an
 * explicit setting wins, then a locally built binary under the project's Cargo
 * target directories, then `PATH`.
 */
object CandelaLspBinary {

    const val NAME: String = "candela-lsp"

    /** The command line the docs tell you to run to get a usable server. */
    const val BUILD_COMMAND: String = "cargo build -p $NAME"

    /**
     * @param command the command to spawn, absolute when a file was found.
     * @param found whether a real file backs the command.
     */
    data class Resolved(val command: String, val found: Boolean)

    fun resolve(project: Project): Resolved {
        val settings = CandelaSettings.getInstance(project).state

        val explicit = expandHome(settings.serverPath.trim())
        if (explicit.isNotEmpty()) {
            return Resolved(explicit, Files.isRegularFile(Paths.get(explicit)))
        }

        if (settings.autoDiscover) {
            for (candidate in targetCandidates(project)) {
                if (Files.isRegularFile(candidate)) {
                    return Resolved(candidate.toString(), true)
                }
            }
        }

        val onPath = PathEnvironmentVariableUtil.findInPath(executableName())
        return if (onPath != null) Resolved(onPath.absolutePath, true) else Resolved(NAME, false)
    }

    private fun executableName(): String = if (SystemInfo.isWindows) "$NAME.exe" else NAME

    /** `target/{debug,embed}/candela-lsp` under every root the project knows about. */
    private fun targetCandidates(project: Project): List<Path> {
        val roots = LinkedHashSet<Path>()

        System.getenv("CARGO_TARGET_DIR")?.takeIf { it.isNotBlank() }?.let { roots.add(Paths.get(it)) }
        project.basePath?.let { roots.add(Paths.get(it, "target")) }
        for (contentRoot in ProjectRootManager.getInstance(project).contentRoots) {
            contentRoot.canonicalPath?.let { roots.add(Paths.get(it, "target")) }
        }

        val binary = executableName()
        return roots.flatMap { PROFILES.map { profile -> it.resolve(profile).resolve(binary) } }
    }

    /**
     * `dev` first, because that is the build the docs tell you to make, then
     * `embed`. `release` is left out on purpose: it sets `panic = "abort"`, and
     * the server relies on catching the compiler's panic to turn it into a
     * diagnostic, so a release build dies on the first error in your buffer.
     */
    private val PROFILES = listOf("debug", "embed")

    private fun expandHome(path: String): String = when {
        path == "~" -> System.getProperty("user.home")
        path.startsWith("~/") || path.startsWith("~\\") ->
            Paths.get(System.getProperty("user.home"), path.substring(2)).toString()
        else -> path
    }
}
