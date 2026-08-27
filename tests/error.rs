use std::error::Error as _;

use goatd::Error;

#[test]
fn errors_with_context_display_that_context_verbatim() {
    for error in [
        Error::InvalidInput("bad parameter".into()),
        Error::Parse("bad graph".into()),
        Error::InvalidDecomposition("bad bags".into()),
        Error::TooLarge("too many vertices".into()),
    ] {
        let expected = match &error {
            Error::InvalidInput(message)
            | Error::Parse(message)
            | Error::InvalidDecomposition(message)
            | Error::TooLarge(message) => message,
            Error::NoDecomposition => unreachable!(),
            _ => unreachable!("the test constructs only errors with stored context"),
        };
        assert_eq!(error.to_string(), expected.as_str());
        assert!(error.source().is_none());
    }
}

#[test]
fn a_missing_flowcutter_result_has_a_stable_public_message() {
    let error = Error::NoDecomposition;

    assert_eq!(error.to_string(), "FlowCutter returned no decomposition");
    assert!(error.source().is_none());
}
