//! `shell::quote_windows`: fixed cases plus a round trip through a `CommandLineToArgvW` parser.

use termigator_core::shell::{join_windows, quote_windows};

/// The argument-splitting rules of `CommandLineToArgvW` for arguments after the program name.
fn parse(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let (mut args, mut i) = (Vec::new(), 0);
    while i < chars.len() {
        while i < chars.len() && matches!(chars[i], ' ' | '\t') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let (mut arg, mut in_quotes) = (String::new(), false);
        while i < chars.len() && (in_quotes || !matches!(chars[i], ' ' | '\t')) {
            let mut backslashes = 0;
            while i < chars.len() && chars[i] == '\\' {
                backslashes += 1;
                i += 1;
            }
            if i < chars.len() && chars[i] == '"' {
                arg.extend(std::iter::repeat_n('\\', backslashes / 2));
                if backslashes % 2 == 1 {
                    arg.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
                i += 1;
            } else {
                arg.extend(std::iter::repeat_n('\\', backslashes));
                if i < chars.len() && (in_quotes || !matches!(chars[i], ' ' | '\t')) {
                    arg.push(chars[i]);
                    i += 1;
                }
            }
        }
        args.push(arg);
    }
    args
}

#[test]
fn fixed_cases() {
    for (arg, want) in [
        ("", r#""""#),
        ("plain", "plain"),
        (r"C:\Windows\System32", r"C:\Windows\System32"),
        ("two words", r#""two words""#),
        ("tab\there", "\"tab\there\""),
        (r#"say "hi""#, r#""say \"hi\"""#),
        (r"C:\Program Files\x", r#""C:\Program Files\x""#),
        (r"trailing \", r#""trailing \\""#),
        (r"a\\ b\", r#""a\\ b\\""#),
        (r#"q\"x"#, r#""q\\\"x""#),
        ("zażółć gęślą", "\"zażółć gęślą\""),
        ("日本語", "日本語"),
    ] {
        assert_eq!(quote_windows(arg), want, "{arg:?}");
    }
}

#[test]
fn round_trips_through_commandlinetoargv() {
    let args = [
        "",
        "a",
        " ",
        "\"",
        "\\",
        "\\\\",
        "\"\"",
        "a b",
        "a\\",
        "a b\\",
        "a b\\\\",
        "\\\" ",
        "\\\\\"",
        "x\"y z\\\"w\\",
        "\t",
        "C:\\Program Files\\Termigator\\",
        "--opt=va lue",
        "łódź \"2\"",
    ];
    let line = join_windows(&args);
    assert_eq!(parse(&line), args);
    for a in args {
        assert_eq!(parse(&quote_windows(a)), [a], "{a:?}");
    }
}
