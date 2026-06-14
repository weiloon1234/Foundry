#[test]
fn raw_string_semantic_identifiers_are_rejected() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/raw_string_semantic_ids.rs");
}

#[test]
fn numeric_columns_reject_string_shortcuts() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/numeric_string_shortcuts.rs");
}

#[test]
fn docs_and_examples_use_table_model_attribute() {
    let roots = ["README.md", "docs", "examples", "tests"];
    let mut offenders = Vec::new();

    for root in roots {
        collect_model_attribute_offenders(std::path::Path::new(root), &mut offenders);
    }

    assert!(
        offenders.is_empty(),
        "{}",
        table_attribute_message(&offenders)
    );
}

fn collect_model_attribute_offenders(path: &std::path::Path, offenders: &mut Vec<String>) {
    if path.is_file() {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            return;
        };
        if !matches!(extension, "md" | "rs" | "stderr") {
            return;
        }
        let contents = std::fs::read_to_string(path).unwrap();
        let legacy = format!("#[{}{} =", "foundry(", "model");
        if contents.contains(&legacy) {
            offenders.push(path.display().to_string());
        }
        return;
    }

    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        collect_model_attribute_offenders(&entry.path(), offenders);
    }
}

fn table_attribute_message(offenders: &[String]) -> String {
    format!(
        "use #[foundry(table = ...)] instead of the legacy model attribute in:\n{}",
        offenders.join("\n")
    )
}
