//! Emit native document/session observations for the pinned editor reference.
use anyhow::{Result, ensure};
use serde_json::Value;
#[path = "../tests/support/snippets.rs"]
mod snippets;

fn main() -> Result<()> {
    let insertion = std::env::args().any(|arg| arg == "--insertion");
    let variables = std::env::args().any(|arg| arg == "--variables");
    let cases: Vec<Value> = serde_json::from_str(if variables {
        include_str!("../tests/vscode-reference/snippet-variable-cases.json")
    } else if insertion {
        include_str!("../tests/vscode-reference/snippet-insertion-cases.json")
    } else {
        include_str!("../tests/vscode-reference/snippet-cases.json")
    })?;
    let observations = snippets::trace(&cases)?;
    if let Some(reference) = std::env::args_os()
        .skip(1)
        .find(|arg| arg != "--insertion" && arg != "--variables")
    {
        let reference: Vec<Value> = serde_json::from_slice(&std::fs::read(reference)?)?;
        ensure!(
            reference.len() == observations.len(),
            "Snippet reference cases disappeared"
        );
        for (actual, expected) in observations.iter().zip(reference) {
            ensure!(
                actual["name"] == expected["name"] && actual["body"] == expected["body"],
                "Snippet fixtures differ"
            );
            ensure!(
                actual["observations"] == expected["observations"],
                "Snippet trace differs for {}: native={}, reference={}",
                actual["name"],
                actual["observations"],
                expected["observations"]
            );
        }
    }
    println!("{}", serde_json::to_string_pretty(&observations)?);
    Ok(())
}
