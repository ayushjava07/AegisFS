use runvane::cli::completion::{generate_completion, Shell};

#[test]
fn test_bash_completion_script_structure() {
    let script = generate_completion(Shell::Bash);
    assert!(script.contains("_runvane()"));
    assert!(script.contains("complete -F _runvane runvane"));
    assert!(script.contains("serve"));
    assert!(script.contains("workflows"));
    assert!(script.contains("runs"));
    assert!(script.contains("dry-run"));
    assert!(script.contains("stats"));
    assert!(script.contains("completion"));
    assert!(script.contains("--endpoint"));
    assert!(script.contains("--format"));
}

#[test]
fn test_zsh_completion_script_structure() {
    let script = generate_completion(Shell::Zsh);
    assert!(script.contains("#compdef runvane"));
    assert!(script.contains("_runvane()"));
    assert!(script.contains("_arguments"));
    assert!(script.contains("serve:Start HTTP server"));
    assert!(script.contains("workflows:Inspect and mutate workflow definitions"));
    assert!(script.contains("completion:Generate shell autocompletion script"));
    assert!(script.contains("--endpoint[Control plane gRPC endpoint]"));
}

#[test]
fn test_fish_completion_script_structure() {
    let script = generate_completion(Shell::Fish);
    assert!(script.contains("complete -c runvane -f"));
    assert!(script.contains("complete -c runvane -n \"__fish_use_subcommand\" -a \"serve\""));
    assert!(script.contains("complete -c runvane -n \"__fish_use_subcommand\" -a \"workflows\""));
    assert!(script.contains("complete -c runvane -n \"__fish_use_subcommand\" -a \"completion\""));
    assert!(script.contains("complete -c runvane -l endpoint"));
    assert!(script.contains("complete -c runvane -l version"));
}

#[test]
fn test_shell_parsing_from_str() {
    assert_eq!("bash".parse::<Shell>().unwrap(), Shell::Bash);
    assert_eq!("BASH".parse::<Shell>().unwrap(), Shell::Bash);
    assert_eq!("zsh".parse::<Shell>().unwrap(), Shell::Zsh);
    assert_eq!("ZSH".parse::<Shell>().unwrap(), Shell::Zsh);
    assert_eq!("fish".parse::<Shell>().unwrap(), Shell::Fish);
    assert_eq!("FISH".parse::<Shell>().unwrap(), Shell::Fish);
    assert!("powershell".parse::<Shell>().is_err());
    assert!("unknown".parse::<Shell>().is_err());
}
