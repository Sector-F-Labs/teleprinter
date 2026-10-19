//! The command line, for what opens Teleprinter as the desktop's terminal:
//! `-e program args…` runs a program in the first pane instead of the login
//! shell (as launchers do for apps that run in a terminal), and
//! `--working-directory dir` starts it there. Without one, the first pane
//! starts where Teleprinter was started (a file manager's "Open Terminal
//! Here"), or at home when that's `/`.

use std::path::PathBuf;

pub const USAGE: &str = "\
usage: teleprinter [--working-directory DIR] [-e PROGRAM [ARGS...]]
       teleprinter msg [--socket PATH] <command>";

/// What the first pane runs, and where.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Launch {
    pub command: Option<(String, Vec<String>)>,
    pub cwd: Option<PathBuf>,
}

/// Reads `args` (without the program's name). `-e` takes everything after
/// it, as xterm's does. An empty error is a request for help.
pub fn parse(args: &[String]) -> Result<Launch, String> {
    let mut launch = Launch::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "-e" | "--command" => {
                let program = args.next().ok_or("-e needs a program to run")?;
                launch.command = Some((program.clone(), args.cloned().collect()));
                break;
            }
            "--working-directory" => {
                let dir = args.next().ok_or("--working-directory needs a directory")?;
                launch.cwd = Some(PathBuf::from(dir));
            }
            other => match other.strip_prefix("--working-directory=") {
                Some(dir) => launch.cwd = Some(PathBuf::from(dir)),
                None => return Err(format!("unknown option {other:?}")),
            },
        }
    }
    Ok(launch)
}

/// Where the first pane starts: as asked, else Teleprinter's own directory,
/// unless that's `/` (as when started by something with nowhere better),
/// where home is more use.
pub fn first_directory(asked: Option<PathBuf>, own: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    asked.or(own.filter(|d| d.as_os_str() != "/")).or(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn e_takes_the_rest_of_the_line() {
        let launch = parse(&args("--working-directory /tmp -e htop -d 10")).unwrap();
        assert_eq!(launch.command, Some(("htop".into(), vec!["-d".into(), "10".into()])));
        assert_eq!(launch.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(parse(&args("--working-directory=/srv")).unwrap().cwd, Some(PathBuf::from("/srv")));
        assert_eq!(parse(&[]).unwrap(), Launch::default());
    }

    #[test]
    fn bad_lines_say_why() {
        assert!(parse(&args("-e")).is_err());
        assert!(parse(&args("--working-directory")).is_err());
        assert!(parse(&args("--frobnicate")).is_err());
    }

    #[test]
    fn the_first_pane_starts_where_asked_else_here_else_home() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(first_directory(p("/tmp"), p("/srv"), p("/home/a")), p("/tmp"));
        assert_eq!(first_directory(None, p("/srv"), p("/home/a")), p("/srv"));
        assert_eq!(first_directory(None, p("/"), p("/home/a")), p("/home/a"));
        assert_eq!(first_directory(None, None, p("/home/a")), p("/home/a"));
    }
}
