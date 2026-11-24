use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parses_positional_values_switches_and_quoted_arguments() {
    let parsed = parse(
        "'team review' item --reviewer user --speed=fast --clear --no-colors -- --literal",
        &["--reviewer", "--speed"],
        &["--clear"],
        &["colors"],
    )
    .expect("valid command arguments");

    assert_eq!(
        parsed,
        ParsedCommandArgs {
            positionals: vec!["team review".into(), "item".into(), "--literal".into()],
            named: vec![
                NamedArgument {
                    name: "--reviewer".into(),
                    value: NamedArgumentValue::Value("user".into()),
                },
                NamedArgument {
                    name: "--speed".into(),
                    value: NamedArgumentValue::Value("fast".into()),
                },
                NamedArgument {
                    name: "--clear".into(),
                    value: NamedArgumentValue::Switch,
                },
                NamedArgument {
                    name: "--colors".into(),
                    value: NamedArgumentValue::Boolean { value: false },
                },
            ],
        }
    );
    assert_eq!(parsed.value("--reviewer"), Some("user"));
    assert!(parsed.has_switch("--clear"));
    assert_eq!(parsed.boolean("colors"), Some(false));
}

#[test]
fn rejects_invalid_named_arguments() {
    for (args, expected) in [
        (
            "--reviewer",
            "Command setting `--reviewer` requires a value.",
        ),
        (
            "--clear=yes",
            "Command setting `--clear` does not take a value.",
        ),
        (
            "--clear --clear",
            "Command setting `--clear` was provided more than once.",
        ),
        (
            "--colors --no-colors",
            "Specify only one of --colors or --no-colors.",
        ),
        ("--unknown", "Unknown command setting `--unknown`."),
        (
            "'unfinished",
            "Invalid command arguments. Check the quoting.",
        ),
    ] {
        assert_eq!(
            parse(args, &["--reviewer"], &["--clear"], &["colors"]),
            Err(expected.to_string())
        );
    }
}
