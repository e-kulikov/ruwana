mod args;
mod output;

use args::{Cli, Command, FormatArg, ListArgs, SortArg, TargetArgs};
use chrono::{DateTime, FixedOffset, Local};
use clap::Parser;
use clap::error::ErrorKind;
use ruwana_core::query::{Filter, SortKey, StatusFilter};
use ruwana_core::resolve::{Selector, parse_id_selector};
use ruwana_core::store::{Store, Warning};
use ruwana_core::{Error, dates, ops};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

/// Thin CLI-level error. Core `Error`s wrap in `Core` (Display verbatim);
/// `Plain` carries CLI-only messages whose text must not gain the
/// `invalid {field}:` prefix that `Error::InvalidField` would add.
enum CliError {
    Core(Error),
    Plain(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Core(e) => write!(f, "{e}"),
            CliError::Plain(m) => f.write_str(m),
        }
    }
}

impl From<Error> for CliError {
    fn from(e: Error) -> Self {
        CliError::Core(e)
    }
}

fn main() -> ExitCode {
    // Spec (Error Handling): ALL user-facing errors exit 1 — including
    // argument-parsing failures, where clap would default to exit 2.
    // `--help`/`--version` arrive here as Err too and must stay exit 0.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            e.print().ok();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprint!("{e}"); // clap errors render their own trailing newline
            return ExitCode::from(1);
        }
    };
    let store = Store::new(wiki_root());
    let now = Local::now().fixed_offset();
    match run(cli.command, &store, now) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// WIKI_ROOT env var, defaulting to ~/wiki (spec: Storage Layout).
fn wiki_root() -> PathBuf {
    std::env::var_os("WIKI_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("wiki")
        })
}

fn print_warnings(warnings: &[Warning]) {
    for w in warnings {
        eprintln!("{w}");
    }
}

/// Build the core Selector from positional/--id, enforcing the spec's
/// mutual-exclusion rule with its exact message.
fn selector(target: &TargetArgs) -> Result<Selector, CliError> {
    match (&target.id_or_title, &target.id) {
        (Some(_), Some(_)) => Err(CliError::Plain(
            "--id and a positional id/title are mutually exclusive".into(),
        )),
        (Some(arg), None) => Ok(Selector::IdOrTitle(arg.clone())),
        (None, Some(id)) => Ok(Selector::Id(parse_id_selector(id)?)),
        (None, None) => Err(CliError::Plain("expected a task id/title or --id".into())),
    }
}

fn parse_instant(input: &str, now: DateTime<FixedOffset>) -> Result<DateTime<FixedOffset>, Error> {
    dates::parse_to_instant(input, now, &Local)
}

fn build_filter(args: &ListArgs, now: DateTime<FixedOffset>) -> Result<Filter, CliError> {
    let status = if args.done {
        StatusFilter::Done
    } else if args.all {
        StatusFilter::All
    } else if args.overdue {
        StatusFilter::Overdue
    } else if args.urgent {
        StatusFilter::Urgent
    } else {
        StatusFilter::Open // --open / --incomplete / default
    };
    Ok(Filter {
        status,
        tags: args.tags.clone(),
        sources: args.sources.clone(),
        due_on: args
            .due
            .as_deref()
            .map(|d| dates::parse_date(d, now))
            .transpose()?,
        created_before: args
            .created_before
            .as_deref()
            .map(|d| parse_instant(d, now))
            .transpose()?,
        created_after: args
            .created_after
            .as_deref()
            .map(|d| parse_instant(d, now))
            .transpose()?,
        modified_before: args
            .modified_before
            .as_deref()
            .map(|d| parse_instant(d, now))
            .transpose()?,
        modified_after: args
            .modified_after
            .as_deref()
            .map(|d| parse_instant(d, now))
            .transpose()?,
    })
}

fn sort_key(sort: SortArg) -> SortKey {
    match sort {
        SortArg::Due => SortKey::Due,
        SortArg::Created => SortKey::Created,
        SortArg::Modified => SortKey::Modified,
        SortArg::Title => SortKey::Title,
    }
}

/// Prompt for rm. Non-interactive without --force is a hard error so
/// agents never hang on a prompt (spec: ruwana rm).
fn confirm_rm(id: &str, title: &str, force: bool) -> Result<bool, CliError> {
    if force {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::Plain(
            "refusing to delete without --force in non-interactive mode".into(),
        ));
    }
    eprint!("delete task {id} \"{title}\"? [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

fn run(command: Command, store: &Store, now: DateTime<FixedOffset>) -> Result<(), CliError> {
    match command {
        Command::Add(a) => {
            let new = ops::NewTask {
                title: a.title,
                description: a.description,
                due: a
                    .due
                    .as_deref()
                    .map(|d| parse_instant(d, now))
                    .transpose()?,
                tags: a.tags,
                sources: a.sources,
                subtasks: a.subtasks,
            };
            let task = ops::add(store, &a.project, new, now)?;
            println!("{}", task.id);
        }
        Command::Done(t) => {
            let report = ops::set_done(store, &selector(&t)?, t.project.as_deref(), true, now)?;
            print_warnings(&report.warnings);
            println!("{}", output::action_line("done", &report));
        }
        Command::Undone(t) => {
            let report = ops::set_done(store, &selector(&t)?, t.project.as_deref(), false, now)?;
            print_warnings(&report.warnings);
            println!("{}", output::action_line("undone", &report));
        }
        Command::List(a) => {
            let q = ops::ListQuery {
                project: a.project.clone(),
                filter: build_filter(&a, now)?,
                sort: sort_key(a.sort),
            };
            let (records, warnings) = ops::list(store, &q, now, &Local)?;
            print_warnings(&warnings);
            match a.format {
                FormatArg::Text => {
                    let text = output::list_text(&records, now);
                    if !text.is_empty() {
                        println!("{text}");
                    }
                }
                FormatArg::Json => println!("{}", output::list_json(&records)),
            }
        }
        Command::Show(a) => {
            let (resolved, raw) = ops::show(
                store,
                &Selector::IdOrTitle(a.lookup.id_or_title.clone()),
                a.lookup.project.as_deref(),
            )?;
            print_warnings(&resolved.warnings);
            match a.format {
                FormatArg::Text => print!("{raw}"),
                FormatArg::Json => println!("{}", output::show_json(&resolved.record)),
            }
        }
        Command::Tasks(a) => {
            let resolved = ops::find(
                store,
                &Selector::IdOrTitle(a.id_or_title.clone()),
                a.project.as_deref(),
            )?;
            print_warnings(&resolved.warnings);
            let text = output::subtasks_text(&resolved.record.task.tasks);
            if !text.is_empty() {
                println!("{text}");
            }
        }
        Command::Edit(a) => {
            let fields = ops::EditFields {
                title: a.title,
                description: a.description,
                due: a
                    .due
                    .as_deref()
                    .map(|d| parse_instant(d, now))
                    .transpose()?,
                add_tags: a.add_tags,
                remove_tags: a.remove_tags,
                add_sources: a.add_sources,
                remove_sources: a.remove_sources,
            };
            let report = ops::edit(
                store,
                &selector(&a.target)?,
                a.target.project.as_deref(),
                fields,
                now,
            )?;
            print_warnings(&report.warnings);
            println!("{}", output::action_line("edited", &report));
        }
        Command::Rm(a) => {
            let resolved = ops::find(store, &selector(&a.target)?, a.target.project.as_deref())?;
            print_warnings(&resolved.warnings);
            let (id, title) = (
                resolved.record.task.id.clone(),
                resolved.record.task.title.clone(),
            );
            if confirm_rm(&id, &title, a.force)? {
                let report = ops::remove(store, &resolved, now)?;
                println!("{}", output::action_line("removed", &report));
            }
        }
    }
    Ok(())
}
