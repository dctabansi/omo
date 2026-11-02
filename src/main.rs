mod buffer;
mod config;
mod editor;
mod terminal;

use anyhow::{Context, Result, bail};
use buffer::Buffer;
use config::{Config, DEFAULT_CONFIG_TEMPLATE};
use editor::Editor;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use terminal::Terminal;

#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    Run { filename: Option<String> },
    OpenConfig,
    ResetDefaultConfig,
    ExportConfig { path: String },
    ImportConfig { path: String },
    PrintHelp,
    PrintVersion,
}

fn parse_cli_args(args: &[String]) -> Result<CliAction> {
    let mut filename: Option<String> = None;
    let mut positional_only = false;
    let mut iter = args.iter().skip(1).peekable();

    while let Some(arg) = iter.next() {
        if !positional_only {
            if arg == "--" {
                positional_only = true;
                continue;
            }
            if arg == "-h" || arg == "--help" {
                return Ok(CliAction::PrintHelp);
            }
            if arg == "--version" || arg == "-v" {
                return Ok(CliAction::PrintVersion);
            }
            if arg == "--config" || arg == "-c" {
                if iter.peek().is_some() || filename.is_some() {
                    bail!("'--config' does not take additional arguments. Usage: omo --config");
                }
                return Ok(CliAction::OpenConfig);
            }
            if arg == "--default" || arg == "-d" {
                if iter.peek().is_some() || filename.is_some() {
                    bail!("'--default' does not take additional arguments. Usage: omo --default");
                }
                return Ok(CliAction::ResetDefaultConfig);
            }
            if arg == "--export" || arg == "-e" {
                if filename.is_some() {
                    bail!("Unexpected argument before '{arg}'. Usage: omo --export [FILE.toml]");
                }
                let path = iter
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "omo-config.toml".to_string());
                if iter.peek().is_some() {
                    bail!("Too many arguments for '{arg}'. Usage: omo --export [FILE.toml]");
                }
                return Ok(CliAction::ExportConfig { path });
            }
            if arg == "--import" || arg == "-i" {
                if filename.is_some() {
                    bail!("Unexpected argument before '{arg}'. Usage: omo --import <FILE.toml>");
                }
                let Some(path) = iter.next().cloned() else {
                    bail!("Missing file path for '{arg}'. Usage: omo --import <FILE.toml>");
                };
                if iter.peek().is_some() {
                    bail!("Too many arguments for '{arg}'. Usage: omo --import <FILE.toml>");
                }
                return Ok(CliAction::ImportConfig { path });
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

fn confirm_prompt(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    Ok(trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes"))
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let filename = match parse_cli_args(&args)? {
        CliAction::PrintHelp => {
            println!(
                "omo {} — One Mode Only terminal text editor\n\nUsage:\n  omo [FILE]\n\nOptions:\n  -c, --config             Open configuration file in omo\n  -d, --default            Restore default configuration (with confirmation)\n  -e, --export [FILE.toml] Export active configuration to a TOML file\n  -i, --import <FILE.toml> Import configuration from a TOML file (with confirmation)\n  -h, --help               Print help\n  -v, --version            Print version",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        CliAction::PrintVersion => {
            println!("omo {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliAction::OpenConfig => {
            let config_path = Config::primary_config_path();
            if !config_path.exists() {
                ensure_parent_dir(&config_path)?;
                fs::write(&config_path, DEFAULT_CONFIG_TEMPLATE).with_context(|| {
                    format!(
                        "Failed to write default config to {}",
                        config_path.display()
                    )
                })?;
            }
            Some(config_path.to_string_lossy().to_string())
        }
        CliAction::ResetDefaultConfig => {
            let config_path = Config::primary_config_path();
            let prompt = format!(
                "Are you sure you want to overwrite {} with default settings?",
                config_path.display()
            );
            if confirm_prompt(&prompt)? {
                ensure_parent_dir(&config_path)?;
                fs::write(&config_path, DEFAULT_CONFIG_TEMPLATE).with_context(|| {
                    format!(
                        "Failed to write default config to {}",
                        config_path.display()
                    )
                })?;
                println!(
                    "Restored default configuration at {}.",
                    config_path.display()
                );
            } else {
                println!("Restore default configuration canceled.");
            }
            return Ok(());
        }
        CliAction::ExportConfig { path } => {
            let export_path = Path::new(&path);
            ensure_parent_dir(export_path)?;
            let primary = Config::primary_config_path();
            let output = if let Ok(existing) = fs::read_to_string(&primary)
                && Config::from_toml_str(&existing).is_ok()
            {
                existing
            } else {
                let (cfg, _) = Config::load_with_warning();
                cfg.to_toml_string()
                    .unwrap_or_else(|_| DEFAULT_CONFIG_TEMPLATE.to_string())
            };
            fs::write(export_path, output).with_context(|| {
                format!(
                    "Failed to export configuration to {}",
                    export_path.display()
                )
            })?;
            println!("Exported configuration to {}.", export_path.display());
            return Ok(());
        }
        CliAction::ImportConfig { path } => {
            let import_path = Path::new(&path);
            let contents = fs::read_to_string(import_path).with_context(|| {
                format!(
                    "Failed to read configuration file {}",
                    import_path.display()
                )
            })?;
            Config::from_toml_str(&contents).with_context(|| {
                format!("Invalid TOML configuration in {}", import_path.display())
            })?;

            let target_path = Config::primary_config_path();
            let prompt = format!(
                "Are you sure you want to overwrite {} with settings from {}?",
                target_path.display(),
                import_path.display()
            );
            if confirm_prompt(&prompt)? {
                ensure_parent_dir(&target_path)?;
                fs::write(&target_path, contents).with_context(|| {
                    format!(
                        "Failed to write imported configuration to {}",
                        target_path.display()
                    )
                })?;
                println!(
                    "Imported configuration from {} to {}.",
                    import_path.display(),
                    target_path.display()
                );
            } else {
                println!("Import configuration canceled.");
            }
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
    fn cli_parser_handles_help_version_config_commands_and_filename() {
        let help = vec!["omo".to_string(), "--help".to_string()];
        assert_eq!(parse_cli_args(&help).unwrap(), CliAction::PrintHelp);

        let version = vec!["omo".to_string(), "--version".to_string()];
        assert_eq!(parse_cli_args(&version).unwrap(), CliAction::PrintVersion);

        let short_v = vec!["omo".to_string(), "-v".to_string()];
        assert_eq!(parse_cli_args(&short_v).unwrap(), CliAction::PrintVersion);

        let capital_v = vec!["omo".to_string(), "-V".to_string()];
        assert!(parse_cli_args(&capital_v).is_err());

        let open_cfg = vec!["omo".to_string(), "--config".to_string()];
        assert_eq!(parse_cli_args(&open_cfg).unwrap(), CliAction::OpenConfig);

        let short_cfg = vec!["omo".to_string(), "-c".to_string()];
        assert_eq!(parse_cli_args(&short_cfg).unwrap(), CliAction::OpenConfig);

        let reset_cfg = vec!["omo".to_string(), "-d".to_string()];
        assert_eq!(
            parse_cli_args(&reset_cfg).unwrap(),
            CliAction::ResetDefaultConfig
        );

        let export_default = vec!["omo".to_string(), "--export".to_string()];
        assert_eq!(
            parse_cli_args(&export_default).unwrap(),
            CliAction::ExportConfig {
                path: "omo-config.toml".to_string()
            }
        );

        let export_custom = vec![
            "omo".to_string(),
            "-e".to_string(),
            "backup.toml".to_string(),
        ];
        assert_eq!(
            parse_cli_args(&export_custom).unwrap(),
            CliAction::ExportConfig {
                path: "backup.toml".to_string()
            }
        );

        let import_cfg = vec![
            "omo".to_string(),
            "--import".to_string(),
            "my-config.toml".to_string(),
        ];
        assert_eq!(
            parse_cli_args(&import_cfg).unwrap(),
            CliAction::ImportConfig {
                path: "my-config.toml".to_string()
            }
        );

        let import_missing_arg = vec!["omo".to_string(), "-i".to_string()];
        assert!(parse_cli_args(&import_missing_arg).is_err());

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
