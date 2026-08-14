//! Zed extension for candela: registers the `.cdl` language and starts
//! `candela-lsp` for it.
//!
//! The server is found the way the VS Code extension finds it: an explicit
//! path in Zed settings wins, then `candela-lsp` on `$PATH`, where the
//! toolchain installs it.

use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

const SERVER_NAME: &str = "candela-lsp";

struct CandelaExtension;

impl zed::Extension for CandelaExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let mut args = None;

        if let Ok(settings) = LspSettings::for_worktree(SERVER_NAME, worktree) {
            if let Some(binary) = settings.binary {
                args = binary.arguments;
                if let Some(path) = binary.path {
                    return Ok(zed::Command {
                        command: path,
                        args: args.unwrap_or_default(),
                        env: worktree.shell_env(),
                    });
                }
            }
        }

        let command = worktree.which(SERVER_NAME).ok_or_else(|| {
            format!(
                "{SERVER_NAME} was not found on $PATH. Install the candela \
                 toolchain, or build the server with `cargo build -p candela-lsp` \
                 and set lsp.{SERVER_NAME}.binary.path in your Zed settings."
            )
        })?;

        Ok(zed::Command {
            command,
            args: args.unwrap_or_default(),
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(CandelaExtension);
