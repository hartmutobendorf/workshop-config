mod api;
mod app;
mod cli;
mod definition;
mod theme;

use std::{
    io::{self, IsTerminal},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use api::Store;
use app::{Action, App};
use cli::{Cli, Command};
use definition::Document;
use theme::Palette;

struct Screen {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Screen {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            disable_raw_mode()?;
            return Err(e.into());
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(e) => {
                disable_raw_mode()?;
                execute!(io::stdout(), LeaveAlternateScreen)?;
                Err(e.into())
            }
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        if let Err(error) = disable_raw_mode() {
            eprintln!("Cannot restore terminal mode: {error}");
        }
        if let Err(error) = execute!(io::stdout(), LeaveAlternateScreen) {
            eprintln!("Cannot restore terminal screen: {error}");
        }
    }
}

fn run(args: Cli) -> Result<()> {
    let palette = Palette::new(args.theme, args.color_enabled());
    let (file, check) = match args.command {
        Some(Command::Help { command, all }) => {
            cli::show_help(command.as_deref(), all)?;
            return Ok(());
        }
        Some(Command::Version) => {
            println!("{} {}", cli::command_name(), env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(Command::RemovedStatus { .. }) => {
            unreachable!("removed command handled before execution")
        }
        Some(Command::Edit(options)) => (options.file, false),
        Some(Command::Validate(options)) => (options.file, true),
        None => (
            args.file.unwrap_or_else(|| "workshop.yaml".into()),
            args.check,
        ),
    };
    if check {
        let metadata = std::fs::metadata(&file)
            .with_context(|| format!("Cannot read definition {}", file.display()))?;
        ensure!(
            metadata.is_file(),
            "Definition is not a regular file: {}",
            file.display()
        );
    }
    let document = Document::open(file)?;
    if check {
        if !args.quiet {
            println!(
                "Valid Workshop definition: {} ({})",
                document.definition.name,
                document.path.display()
            );
        }
        return Ok(());
    }
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "The TUI requires an interactive terminal; run '{} validate <file>' for validation",
        cli::command_name()
    );
    let store = Store::new(&args.store_url)?;
    let mut app = App::new(document, store, args.architecture);
    app.set_palette(palette);
    let mut screen = Screen::enter()?;
    loop {
        app.tick();
        screen.terminal.draw(|f| app.draw(f))?;
        if event::poll(Duration::from_millis(100))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match app.key(key) {
                Ok(Action::Quit) => break,
                Ok(Action::Editor) => {
                    drop(screen);
                    let edited = definition::edit_external(&app.document.definition);
                    screen = Screen::enter().context("Cannot restore TUI after editor")?;
                    match edited {
                        Ok(definition) => app.apply_edited(definition),
                        Err(e) => app.set_error(format!("{e:#}")),
                    }
                }
                Ok(Action::Continue) => {}
                Err(e) => app.set_error(format!("{e:#}")),
            }
        }
    }
    drop(screen);
    if let Some(notice) = app.exit_notice() {
        eprintln!("{notice}");
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        previous_hook(info);
    }));
    let args = match Cli::parse_args() {
        Ok(args) => args,
        Err(error) => {
            if let Err(print_error) = error.print() {
                eprintln!("error: cannot print command help: {print_error}");
                return std::process::ExitCode::FAILURE;
            }
            if error.use_stderr() && !error.to_string().contains("Usage:") {
                eprintln!(
                    "\n{}\n\nFor more information, run '{} help <command>'.",
                    cli::command().render_usage(),
                    cli::command_name()
                );
            }
            return std::process::ExitCode::from(error.exit_code() as u8);
        }
    };
    if matches!(args.command, Some(Command::RemovedStatus { .. })) {
        eprintln!(
            "error: '{} status' was removed in 0.2.0. Workshop daemon access is no longer supported.\nUse '{} edit [FILE]' or '{} validate [FILE]' to work with definitions.",
            cli::command_name(),
            cli::command_name(),
            cli::command_name()
        );
        return std::process::ExitCode::from(2);
    }
    match run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let name = cli::command_name();
            eprintln!(
                "error: {error:#}\n\nUsage:\n  {name} edit [FILE]\n  {name} validate [FILE]\n\nFor more information, run '{name} help <command>'."
            );
            if definition::snap_root().is_some()
                && error.chain().any(|cause| {
                    cause.downcast_ref::<io::Error>().is_some_and(|error| {
                        matches!(
                            error.kind(),
                            io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound
                        )
                    })
                })
            {
                eprintln!(
                    "\nStrict snap access is limited to permitted home-directory paths. Use a non-hidden project directory such as ~/projects/my-workshop, and check 'snap connections workshop-config' for the home connection."
                );
            }
            std::process::ExitCode::FAILURE
        }
    }
}
