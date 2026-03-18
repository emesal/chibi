/// Integration test that regenerates the hook reference section in `docs/hooks.md`.
///
/// Run via: `just generate-docs`
/// Which executes: `cargo test -p chibi-core --test generate_docs -- --nocapture`
///
/// The test also acts as a freshness check: if the generated content matches
/// what's in the file, the test passes silently. If not, it updates the file
/// and reports what changed (but still passes, since that's the generator's job).
///
/// The unit test `test_hooks_docs_markdown_freshness` in `eval.rs` is the CI
/// check that fails when the file is stale — it must be run after generate_docs.
use std::path::PathBuf;

#[test]
fn generate_docs() {
    let generated = chibi_core::tools::generate_hooks_markdown();

    let hooks_md_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("docs/hooks.md");

    let hooks_md = std::fs::read_to_string(&hooks_md_path)
        .unwrap_or_else(|_| panic!("could not read {}", hooks_md_path.display()));

    const BEGIN_MARKER: &str =
        "<!-- BEGIN GENERATED HOOK REFERENCE — do not edit, run `just generate-docs` -->";
    const END_MARKER: &str = "<!-- END GENERATED HOOK REFERENCE -->";

    let begin_pos = hooks_md
        .find(BEGIN_MARKER)
        .expect("BEGIN GENERATED marker not found in docs/hooks.md");
    let end_pos = hooks_md
        .find(END_MARKER)
        .expect("END GENERATED marker not found in docs/hooks.md");

    let before = &hooks_md[..begin_pos + BEGIN_MARKER.len()];
    let after = &hooks_md[end_pos..];

    let new_content = format!("{before}\n{generated}\n{after}");

    if hooks_md != new_content {
        std::fs::write(&hooks_md_path, &new_content)
            .unwrap_or_else(|e| panic!("could not write {}: {e}", hooks_md_path.display()));
        println!("docs/hooks.md updated.");
    } else {
        println!("docs/hooks.md is already up-to-date.");
    }
}
