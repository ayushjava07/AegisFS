//! Shell autocompletion script generation for Bash, Zsh, and Fish.

/// Supported target shells for autocompletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// Bourne Again SHell
    Bash,
    /// Z Shell
    Zsh,
    /// Friendly Interactive SHell
    Fish,
}

impl Shell {
    /// Parses shell name from string argument.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }
}

/// Generates shell autocompletion script for the specified shell.
pub fn generate_completion(shell: Shell) -> String {
    match shell {
        Shell::Bash => generate_bash(),
        Shell::Zsh => generate_zsh(),
        Shell::Fish => generate_fish(),
    }
}

fn generate_bash() -> String {
    r#"# Bash completion script for runvane
_runvane() {
    local cur prev words cword
    _init_completion || return

    local subcommands="serve workflows runs dry-run stats completion version help"

    case "${prev}" in
        runvane)
            COMPREPLY=( $(compgen -W "${subcommands}" -- "${cur}") )
            return 0
            ;;
        workflows)
            COMPREPLY=( $(compgen -W "create list get" -- "${cur}") )
            return 0
            ;;
        runs)
            COMPREPLY=( $(compgen -W "submit list get cancel" -- "${cur}") )
            return 0
            ;;
        completion)
            COMPREPLY=( $(compgen -W "bash zsh fish" -- "${cur}") )
            return 0
            ;;
        --format|-f)
            COMPREPLY=( $(compgen -W "text json" -- "${cur}") )
            return 0
            ;;
        *)
            ;;
    esac

    COMPREPLY=( $(compgen -W "--endpoint --help --version" -- "${cur}") )
}
complete -F _runvane runvane
"#
    .to_string()
}

fn generate_zsh() -> String {
    r#"#compdef runvane
# Zsh completion script for runvane

_runvane() {
    local -a commands
    commands=(
        'serve:Start HTTP server, gRPC server, and scheduler worker pool'
        'workflows:Inspect and mutate workflow definitions'
        'runs:Inspect and mutate runs'
        'dry-run:Statically simulate and analyze a workflow DAG without running it'
        'stats:Display real-time control plane health and execution telemetry'
        'completion:Generate shell autocompletion script'
        'version:Print product and build info'
    )

    _arguments \
        '--endpoint[Control plane gRPC endpoint]:URL:_urls' \
        '--help[Print help information]' \
        '--version[Print version information]' \
        '*::command:->subcmd'

    case "$state" in
        subcmd)
            _describe -t commands 'runvane command' commands
            ;;
    esac
}

_runvane "$@"
"#
    .to_string()
}

fn generate_fish() -> String {
    r#"# Fish completion script for runvane

complete -c runvane -f
complete -c runvane -n "__fish_use_subcommand" -a "serve" -d "Start control plane servers and worker pool"
complete -c runvane -n "__fish_use_subcommand" -a "workflows" -d "Inspect and mutate workflow definitions"
complete -c runvane -n "__fish_use_subcommand" -a "runs" -d "Inspect and mutate workflow runs"
complete -c runvane -n "__fish_use_subcommand" -a "dry-run" -d "Statically simulate and analyze a workflow DAG"
complete -c runvane -n "__fish_use_subcommand" -a "stats" -d "Display real-time control plane health and diagnostics"
complete -c runvane -n "__fish_use_subcommand" -a "completion" -d "Generate shell autocompletion script"
complete -c runvane -n "__fish_use_subcommand" -a "version" -d "Print version information"

complete -c runvane -l endpoint -d "Control plane gRPC endpoint" -r
complete -c runvane -l help -d "Print help information"
complete -c runvane -l version -d "Print version information"

complete -c runvane -n "__fish_seen_subcommand_from completion" -a "bash zsh fish"
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_shells() {
        assert_eq!(Shell::parse("bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("ZSH"), Some(Shell::Zsh));
        assert_eq!(Shell::parse("Fish"), Some(Shell::Fish));
        assert_eq!(Shell::parse("unknown"), None);
    }

    #[test]
    fn generate_all_shell_scripts() {
        let bash = generate_completion(Shell::Bash);
        assert!(bash.contains("_runvane()"));
        assert!(bash.contains("dry-run"));

        let zsh = generate_completion(Shell::Zsh);
        assert!(zsh.contains("#compdef runvane"));
        assert!(zsh.contains("_describe"));

        let fish = generate_completion(Shell::Fish);
        assert!(fish.contains("complete -c runvane"));
        assert!(fish.contains("stats"));
    }
}
