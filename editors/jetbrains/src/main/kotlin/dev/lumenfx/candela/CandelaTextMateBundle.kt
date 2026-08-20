package dev.lumenfx.candela

import com.intellij.openapi.application.PathManager
import com.intellij.openapi.diagnostic.logger
import org.jetbrains.plugins.textmate.api.TextMateBundleProvider
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption

/**
 * Hands the IDE the candela TextMate grammar, which is what colors `.cdl`
 * files. The bundle ships inside the plugin jar and is unpacked once per plugin
 * version, because TextMate reads bundles from a directory.
 */
class CandelaTextMateBundle : TextMateBundleProvider {

    override fun getBundles(): List<TextMateBundleProvider.PluginBundle> {
        val directory = unpack() ?: return emptyList()
        return listOf(TextMateBundleProvider.PluginBundle("Candela", directory))
    }

    private fun unpack(): Path? {
        val directory = Path.of(PathManager.getSystemPath(), "candela-textmate", bundleVersion())
        return try {
            for (file in BUNDLE_FILES) {
                val target = directory.resolve(file)
                if (Files.exists(target)) {
                    continue
                }
                Files.createDirectories(target.parent)
                val resource = CandelaTextMateBundle::class.java.getResourceAsStream("$RESOURCES/$file")
                    ?: error("missing bundle resource $file")
                resource.use { Files.copy(it, target, StandardCopyOption.REPLACE_EXISTING) }
            }
            directory
        } catch (e: Exception) {
            LOG.error("Candela: cannot unpack the TextMate bundle to $directory", e)
            null
        }
    }

    /** The build stamps the plugin version into this resource. */
    private fun bundleVersion(): String {
        val resource = CandelaTextMateBundle::class.java.getResourceAsStream("$RESOURCES/bundle-version.txt")
            ?: return "dev"
        return resource.use { it.readBytes().decodeToString() }.trim().ifEmpty { "dev" }
    }

    private companion object {
        const val RESOURCES = "/textmate"

        /** Every file the bundle needs, including the paths named in package.json. */
        val BUNDLE_FILES = listOf(
            "package.json",
            "syntaxes/candela.tmLanguage.json",
            "language-configuration/candela.json",
        )

        val LOG = logger<CandelaTextMateBundle>()
    }
}
