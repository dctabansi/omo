mod buffer;
mod config;
mod editor;
mod terminal;

use anyhow::{Result, bail};
use buffer::Buffer;
use config::Config;
use editor::Editor;
use std::env;
use terminal::Terminal;

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Run { filename: Option<String> },
    PrintHelp,
    PrintVersion,
}

fn parse_cli_args(args: &[String]) -> Result<CliAction> {
    let mut filename: Option<String> = None;
    let mut positional_only = false;

    for arg in args.iter().skip(1) {
        if !positional_only {
            if arg == "--" {
                positional_only = true;
                continue;
            }
            if arg == "-h" || arg == "--help" {
                return Ok(CliAction::PrintHelp);
            }
            if arg == "-v" || arg == "-V" || arg == "--version" {
                return Ok(CliAction::PrintVersion);
            }
            if arg.starts_with('-') && arg.len() > 1 {
                bail!("Unknown option '{arg}'. Try 'omo --help' for usage.");
            }
        }

        if filename.is_some() {
            bail!("Too many arguments. Usage: omo [FILE]");
        }
        filename = Some(arg.clone());
    }

    Ok(CliAction::Run { filename })
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let filename = match parse_cli_args(&args)? {
        CliAction::PrintHelp => {
            println!(
                "omo {} — One Mode Only terminal text editor\n\nUsage:\n  omo [FILE]\n\nOptions:\n  -h, --help     Print help\n  -v, -V, --version  Print version",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        CliAction::PrintVersion => {
            println!("omo {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliAction::Run { filename } => filename,
    };

    let (config, config_warning) = Config::load_with_warning();

    let buffer = if let Some(path) = &filename {
        Buffer::open(path)?
    } else {
        Buffer::new()
    };

    let _term = Terminal::initialize()?;

    let mut editor = Editor::with_config(buffer, config);
    if let Some(warning) = config_warning {
        editor.status_message = Some(warning);
    }
    editor.run()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parser_handles_help_version_and_filename() {
        let help = vec!["omo".to_string(), "--help".to_string()];
        assert_eq!(parse_cli_args(&help).unwrap(), CliAction::PrintHelp);

        let version = vec!["omo".to_string(), "-V".to_string()];
        assert_eq!(parse_cli_args(&version).unwrap(), CliAction::PrintVersion);

        let short_v = vec!["omo".to_string(), "-v".to_string()];
        assert_eq!(parse_cli_args(&short_v).unwrap(), CliAction::PrintVersion);

        let file = vec!["omo".to_string(), "notes.txt".to_string()];
        assert_eq!(
            parse_cli_args(&file).unwrap(),
            CliAction::Run {
                filename: Some("notes.txt".to_string())
            }
        );

        let dash_file = vec!["omo".to_string(), "--".to_string(), "--help".to_string()];
        assert_eq!(
            parse_cli_args(&dash_file).unwrap(),
            CliAction::Run {
                filename: Some("--help".to_string())
            }
        );

        let unknown = vec!["omo".to_string(), "--unknown".to_string()];
        assert!(parse_cli_args(&unknown).is_err());

        let too_many = vec!["omo".to_string(), "a.txt".to_string(), "b.txt".to_string()];
        assert!(parse_cli_args(&too_many).is_err());
    }
}
