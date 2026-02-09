use inquire::{Confirm, CustomUserError, Password, Select, Text};
use std::fmt::Display;

/// Prompt the user to confirm an action
///
/// # Errors
///
/// Returns `InquireError` if the prompt fails or is cancelled.
pub fn confirm(message: &str, default: bool) -> Result<bool, inquire::InquireError> {
    Confirm::new(message).with_default(default).prompt()
}

/// Prompt the user to select from a list of options
///
/// # Errors
///
/// Returns `InquireError` if the prompt fails or is cancelled.
pub fn select_option<T: Display>(
    message: &str,
    options: Vec<T>,
) -> Result<T, inquire::InquireError> {
    Select::new(message, options).prompt()
}

/// Prompt the user for text input
///
/// # Errors
///
/// Returns `InquireError` if the prompt fails or is cancelled.
pub fn text_input(message: &str, default: Option<&str>) -> Result<String, inquire::InquireError> {
    let mut prompt = Text::new(message);
    if let Some(default_value) = default {
        prompt = prompt.with_default(default_value);
    }
    prompt.prompt()
}

/// Prompt the user for text input with suggestions from a predefined list.
/// The user can pick a suggestion (Tab to complete) or type a custom value.
///
/// # Errors
///
/// Returns `InquireError` if the prompt fails or is cancelled.
pub fn text_input_with_suggestions(
    message: &str,
    default: Option<&str>,
    suggestions: Vec<String>,
) -> Result<String, inquire::InquireError> {
    let mut prompt = Text::new(message).with_autocomplete(
        move |input: &str| -> Result<Vec<String>, CustomUserError> {
            let lower = input.to_lowercase();
            Ok(suggestions
                .iter()
                .filter(|s| s.to_lowercase().contains(&lower))
                .cloned()
                .collect())
        },
    );
    if let Some(default_value) = default {
        prompt = prompt.with_default(default_value);
    }
    prompt.prompt()
}

/// Prompt the user for a password/secret input (masked)
///
/// # Errors
///
/// Returns `InquireError` if the prompt fails or is cancelled.
pub fn password_input(message: &str) -> Result<String, inquire::InquireError> {
    Password::new(message).without_confirmation().prompt()
}
