//! Where the app listens: a name only the current user's programs use.

use std::path::PathBuf;

/// Where the open app listens for `kadr-mcp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAddress {
    /// A Windows named pipe, `\\.\pipe\kadr-app-<user>`.
    NamedPipe(String),
    /// A Unix socket file.
    UnixSocket(PathBuf),
}

/// The variable naming another address, for tests and for running two apps side
/// by side while developing: a pipe name on Windows, a socket path elsewhere.
pub const ADDRESS_OVERRIDE_VARIABLE: &str = "KADR_APP_LINK_ADDRESS";

/// The address of the current user's app.
pub fn link_address() -> LinkAddress {
    if let Ok(address) = std::env::var(ADDRESS_OVERRIDE_VARIABLE)
        && !address.trim().is_empty()
    {
        return address_from_text(address.trim());
    }
    default_address(&user_name())
}

fn address_from_text(address: &str) -> LinkAddress {
    if cfg!(windows) {
        LinkAddress::NamedPipe(address.to_string())
    } else {
        LinkAddress::UnixSocket(PathBuf::from(address))
    }
}

/// The address for `user`: a named pipe on Windows; elsewhere a socket in the
/// user's runtime folder, or the temporary folder with the user's name.
fn default_address(user: &str) -> LinkAddress {
    let safe_user: String = user
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cfg!(windows) {
        return LinkAddress::NamedPipe(format!(r"\\.\pipe\kadr-app-{safe_user}"));
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime_folder) if !runtime_folder.is_empty() => {
            LinkAddress::UnixSocket(PathBuf::from(runtime_folder).join("kadr-app.sock"))
        }
        _ => {
            LinkAddress::UnixSocket(std::env::temp_dir().join(format!("kadr-app-{safe_user}.sock")))
        }
    }
}

fn user_name() -> String {
    ["USERNAME", "USER", "LOGNAME"]
        .iter()
        .find_map(|variable| std::env::var(variable).ok().filter(|name| !name.is_empty()))
        .unwrap_or_else(|| "user".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_names_the_user_safely() {
        match default_address("Sam Dupont\\x") {
            LinkAddress::NamedPipe(name) => assert_eq!(name, r"\\.\pipe\kadr-app-Sam_Dupont_x"),
            LinkAddress::UnixSocket(path) => {
                let text = path.display().to_string();
                assert!(
                    text.ends_with("kadr-app.sock") || text.ends_with("kadr-app-Sam_Dupont_x.sock"),
                    "{text}"
                );
            }
        }
    }
}
