//! # converter
//!
//! The module which converts the shell script to windows batch script.
//!

#[cfg(test)]
#[path = "./converter_test.rs"]
mod converter_test;

use regex::Regex;

static SHELL2BATCH_PREFIX: &str = "# shell2batch:";

fn replace_flags(arguments: &str, flags_mappings: Vec<(&str, &str)>) -> String {
    let mut windows_arguments = arguments.to_string();

    for flags in flags_mappings {
        let (shell_flag, windows_flag) = flags;

        windows_arguments = match Regex::new(shell_flag) {
            Ok(shell_regex) => {
                let str_value = &shell_regex.replace_all(&windows_arguments, windows_flag);
                str_value.to_string()
            }
            Err(_) => windows_arguments,
        };
    }

    windows_arguments
}

fn convert_var<'a>(value: &'a str, buffer: &mut Vec<&'a str>) {
    // Batch file vars have one of two forms: `%NAME%` (corresponding to regular variables),
    // or `%n` if `n` is a digit in the range 0 to 9 or an `*` (corresponding to input params).
    match value {
        "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
            buffer.push("%");
            buffer.push(value);
        }
        "@" => buffer.push("%*"),
        _ => {
            buffer.push("%");
            buffer.push(value);
            buffer.push("%");
        }
    }
}

fn replace_full_vars(arguments: &str) -> String {
    let mut parts: Vec<&str> = arguments.split("${").collect();
    let mut buffer = vec![];

    buffer.push(parts.remove(0));

    for part in parts {
        let (before, after, found) = match part.find("}") {
            None => (part, "", false),
            Some(index) => {
                let values = part.split_at(index);

                (values.0, &values.1[1..values.1.len()], true)
            }
        };

        if found {
            convert_var(before, &mut buffer);
        } else {
            buffer.push(before)
        }

        if after.len() > 0 {
            buffer.push(after);
        }
    }

    buffer.join("").to_string()
}

fn replace_partial_vars(arguments: &str) -> String {
    let mut parts: Vec<&str> = arguments.split('$').collect();
    let mut buffer = vec![];

    buffer.push(parts.remove(0));

    for part in parts {
        let (before, after) = match part.find(" ") {
            None => (part, ""),
            Some(index) => part.split_at(index),
        };

        convert_var(before, &mut buffer);

        if after.len() > 0 {
            buffer.push(after);
        }
    }

    buffer.join("").to_string()
}

fn replace_vars(arguments: &str) -> String {
    let mut updated_arguments = replace_full_vars(arguments);
    updated_arguments = replace_partial_vars(&updated_arguments);

    updated_arguments
}

fn add_arguments(arguments: &str, additional_arguments: Vec<String>, pre: bool) -> String {
    let mut windows_arguments = if pre {
        "".to_string()
    } else {
        arguments.to_string()
    };

    for additional_argument in additional_arguments {
        windows_arguments.push_str(&additional_argument);
    }

    if pre {
        if arguments.len() > 0 {
            windows_arguments.push_str(" ");
        }
        windows_arguments.push_str(arguments);
    }

    windows_arguments.trim_start().to_string()
}

fn convert_line(line: &str) -> String {
    if line.contains(SHELL2BATCH_PREFIX) {
        let index = line.find(SHELL2BATCH_PREFIX).unwrap() + SHELL2BATCH_PREFIX.len();
        let windows_command = line[index..].trim();
        windows_command.to_string()
    } else if line.starts_with("#") {
        let mut windows_command = String::from(line);
        windows_command.remove(0);
        windows_command.insert_str(0, "@REM ");

        windows_command
    } else {
        // assume first word is the command
        let (shell_command, mut arguments) = match line.find(" ") {
            None => (line, "".to_string()),
            Some(index) => {
                let (shell_command, arguments_str) = line.split_at(index);

                (shell_command, arguments_str.to_string())
            }
        };

        arguments = arguments.trim().to_string();

        let (
            mut windows_command,
            flags_mappings,
            pre_arguments,
            post_arguments,
            modify_path_separator,
        ) = match shell_command {
            "cp" => {
                // There is no good `cp` equivalent on windows. There are
                // two tools we can rely on:
                //
                // - xcopy, which is great for directory to directory
                //   copies.
                // - copy, which is great for file to file/directory copies.
                //
                // We can select which one to use based on the presence of
                // the -r flag.
                let win_cmd = match Regex::new("(^|\\s)-[^ ]*[rR]") {
                    Ok(regex_instance) => {
                        if regex_instance.is_match(&arguments) {
                            "xcopy".to_string()
                        } else {
                            "copy".to_string()
                        }
                    }
                    Err(_) => "copy".to_string(),
                };

                let flags_mappings = if win_cmd == "xcopy".to_string() {
                    vec![("-[rR]", "/E")]
                } else {
                    vec![]
                };
                (win_cmd, flags_mappings, vec![], vec![], true)
            }
            "mv" => ("move".to_string(), vec![], vec![], vec![], true),
            "ls" => ("dir".to_string(), vec![], vec![], vec![], true),
            "rm" => {
                let win_cmd = match Regex::new("-[a-zA-Z]*[rR][a-zA-Z]* ") {
                    Ok(regex_instance) => {
                        if regex_instance.is_match(&arguments) {
                            "rmdir".to_string()
                        } else {
                            "del".to_string()
                        }
                    }
                    Err(_) => "del".to_string(),
                };

                let flags_mappings = if win_cmd == "rmdir".to_string() {
                    vec![("-([rR][fF]|[fF][rR]) ", "/S /Q "), ("-[rR]+ ", "/S ")]
                } else {
                    vec![("-[fF] ", "/Q ")]
                };

                // If there is a -f flag, add " 2>nul" to the end of the command to suppress errors,
                //   and add " || cd ." as a lightweight command that always returns a success exit code.
                let post_arguments = match Regex::new(r#"(?i)^-\S*f\S*"#) {
                    Ok(regex_instance) => {
                        if regex_instance.is_match(&arguments) {
                            vec![" 2>nul || cd .".to_owned()]
                        } else {
                            vec![]
                        }
                    }
                    Err(_) => vec![],
                };

                (win_cmd, flags_mappings, vec![], post_arguments, true)
            }
            "mkdir" => (
                "mkdir".to_string(),
                vec![("-[pP]", "")],
                vec![],
                vec![],
                true,
            ),
            "clear" => ("cls".to_string(), vec![], vec![], vec![], false),
            "grep" => ("find".to_string(), vec![], vec![], vec![], false),
            "pwd" => ("chdir".to_string(), vec![], vec![], vec![], false),
            "export" => ("set".to_string(), vec![], vec![], vec![], false),
            "unset" => (
                "set".to_string(),
                vec![],
                vec![],
                vec!["=".to_string()],
                false,
            ),
            "touch" => {
                let mut file_arg = arguments.replace("/", "\\").to_string();
                file_arg.push_str("+,,");

                (
                    "copy".to_string(),
                    vec![],
                    vec!["/B ".to_string(), file_arg.clone()],
                    vec![],
                    true,
                )
            }
            "set" => (
                "@echo".to_string(),
                vec![("-x", "on"), ("\\+x", "off")],
                vec![],
                vec![],
                false,
            ),
            _ => (shell_command.to_string(), vec![], vec![], vec![], false),
        };

        // modify paths
        if modify_path_separator {
            arguments = arguments.replace("/", "\\");
        }
        windows_command = windows_command.replace("/", "\\");

        let mut windows_arguments = arguments.to_string();

        // add pre arguments
        windows_arguments = if pre_arguments.len() > 0 {
            add_arguments(&windows_arguments, pre_arguments, true)
        } else {
            windows_arguments
        };

        // replace flags
        windows_arguments = if flags_mappings.len() > 0 {
            replace_flags(&arguments, flags_mappings)
        } else {
            windows_arguments
        };

        // replace vars
        windows_arguments = if windows_arguments.len() > 0 {
            replace_vars(&windows_arguments)
        } else {
            windows_arguments
        };
        windows_command = replace_vars(&windows_command);

        // add post arguments
        windows_arguments = if post_arguments.len() > 0 {
            add_arguments(&windows_arguments, post_arguments, false)
        } else {
            windows_arguments
        };

        if windows_arguments.len() > 0 {
            windows_command.push_str(" ");
            windows_command.push_str(&windows_arguments);
        }

        windows_command
    }
}

/// Converts the provided shell script and returns the windows batch script text.
pub(crate) fn run(script: &str) -> String {
    let lines: Vec<&str> = script.split('\n').collect();
    let mut windows_batch = vec![];

    for mut line in lines {
        line = line.trim();
        let mut line_string = line.to_string();

        // convert line
        let converted_line = if line_string.len() == 0 {
            line_string
        } else {
            convert_line(&mut line_string)
        };

        windows_batch.push(converted_line);
    }

    windows_batch.join("\n")
}
