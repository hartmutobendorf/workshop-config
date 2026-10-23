use std::{io::IsTerminal, path::PathBuf};

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};

use crate::theme::Theme;

const HELP: &str =
    "{bin} {version}\n\nUsage:\n  {usage}\n\nSummary:\n  {about}\n\n{all-args}{after-help}";
const GUIDANCE: &str = "For more information, run 'workshop-tui help <command>'.\nFor all commands, run 'workshop-tui help --all'.";

#[derive(Parser)]
#[command(
    version,
    propagate_version = true,
    term_width = 100,
    about = "Create and edit Canonical Workshop definitions.",
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true,
    help_template = HELP,
    after_help = "Examples:\n  workshop-tui edit my-workshop.yaml\n  workshop-tui validate workshop.yaml\n\nWith no command, open the editor. A file argument is shorthand for 'edit <file>'.\n\nFor more information, run 'workshop-tui help <command>'.\nFor all commands, run 'workshop-tui help --all'."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Definition to edit (shorthand for 'edit <file>').
    pub file: Option<PathBuf>,
    /// Validate an existing file (shorthand for 'validate <file>').
    #[arg(long)]
    pub check: bool,
    /// SDK Store base URL.
    #[arg(
        long,
        global = true,
        env = "SDK_STORE_URL",
        default_value = "https://api.pkg.store",
        help_heading = "Global options"
    )]
    pub store_url: String,
    /// Target SDK architecture (for example amd64, arm64, or riscv64).
    #[arg(long, global = true, default_value_t = default_arch(), help_heading = "Global options")]
    pub architecture: String,
    /// Interface theme.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value = "light",
        help_heading = "Global options"
    )]
    pub theme: Theme,
    /// Color mode; NO_COLOR always disables color.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value = "auto",
        help_heading = "Global options"
    )]
    pub color: ColorMode,
    /// Suppress success messages, but retain output data and errors.
    #[arg(long, global = true, help_heading = "Global options")]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Open the SDK/toolchain pickers and YAML preview; write only when explicitly saved.
    #[command(help_template = HELP, after_help = "Examples:\n  workshop-tui edit\n  workshop-tui edit my-workshop.yaml --theme dark\n\nRelated commands:\n  validate  Check an existing definition without a terminal or network.\n\nFor more information, run 'workshop-tui help <command>'.")]
    Edit(DefinitionArgs),
    /// Validate an existing definition without a terminal, network, or writes.
    #[command(help_template = HELP, after_help = "Examples:\n  workshop-tui validate workshop.yaml\n  workshop-tui validate .workshop/design.yaml --quiet\n\nRelated commands:\n  edit  Open the interactive definition editor.\n\nFor more information, run 'workshop-tui help <command>'.")]
    Validate(DefinitionArgs),
    #[command(
        name = "status",
        hide = true,
        disable_help_flag = true,
        disable_version_flag = true
    )]
    RemovedStatus {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        _args: Vec<std::ffi::OsString>,
    },
    /// Show release version.
    #[command(help_template = HELP, after_help = GUIDANCE)]
    Version,
    /// Show general help or help for a command.
    #[command(help_template = HELP, after_help = "Examples:\n  workshop-tui help edit\n  workshop-tui help --all\n\nFor more information, run 'workshop-tui help <command>'.")]
    Help {
        /// Command whose help to display.
        #[arg(value_parser = ["edit", "validate", "version", "help"], conflicts_with = "all")]
        command: Option<String>,
        /// Show help for every command.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Args)]
pub struct DefinitionArgs {
    /// Definition file; edit creates it only on save, validate requires it to exist.
    #[arg(default_value = "workshop.yaml")]
    pub file: PathBuf,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl Cli {
    pub fn parse_args() -> Result<Self, clap::Error> {
        let matches = command().try_get_matches()?;
        Self::from_arg_matches(&matches)
    }

    pub fn color_enabled(&self) -> bool {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        match self.color {
            ColorMode::Never => false,
            ColorMode::Always => true,
            ColorMode::Auto => {
                std::io::stdout().is_terminal()
                    && std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb")
            }
        }
    }
}

pub fn command_name() -> &'static str {
    if std::env::args_os().next().is_some_and(|arg| {
        std::path::Path::new(&arg)
            .file_name()
            .is_some_and(|name| name == "workshop-config")
    }) {
        "workshop-config"
    } else {
        "workshop-tui"
    }
}

pub fn command() -> clap::Command {
    let name = command_name();
    let mut root = Cli::command().name(name).bin_name(name);
    if name != "workshop-tui" {
        fn update_help(command: clap::Command, name: &'static str) -> clap::Command {
            let help = command
                .get_after_help()
                .map(|help| help.to_string().replace("workshop-tui", name));
            match help {
                Some(help) => command.after_help(help),
                None => command,
            }
        }
        root = update_help(root, name).mut_subcommands(|subcommand| update_help(subcommand, name));
    }
    root
}

pub fn show_help(command: Option<&str>, all: bool) -> std::io::Result<()> {
    let mut root = self::command();
    root.build();
    if let Some(name) = command {
        root.find_subcommand_mut(name)
            .expect("validated command name")
            .print_help()?;
    } else {
        root.print_help()?;
        if all {
            for command in root.get_subcommands_mut() {
                if command.is_hide_set() {
                    continue;
                }
                println!();
                command.print_help()?;
            }
        }
    }
    Ok(())
}

fn default_arch() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_structure_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn preserves_file_shorthand_and_check_while_supporting_verbs() {
        let cli = Cli::try_parse_from(["workshop-tui", "custom.yaml", "--check"]).unwrap();
        assert_eq!(cli.file.unwrap(), PathBuf::from("custom.yaml"));
        assert!(cli.check);
        let cli = Cli::try_parse_from(["workshop-tui", "edit", "custom.yaml", "--theme", "dark"])
            .unwrap();
        assert!(matches!(cli.command, Some(Command::Edit(_))));
        assert!(Cli::try_parse_from(["workshop-tui", "help", "--all"]).is_ok());
        assert!(Cli::try_parse_from(["workshop-tui", "help", "edit"]).is_ok());
        assert!(Cli::try_parse_from(["workshop-tui", "edit", "--theme", "unknown"]).is_err());
        assert!(matches!(
            Cli::try_parse_from(["workshop-tui", "status", "--socket", "/tmp/test"])
                .unwrap()
                .command,
            Some(Command::RemovedStatus { .. })
        ));
    }
}
