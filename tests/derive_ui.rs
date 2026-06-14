#[test]
fn database_derives_enforce_their_contracts() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/derive/pass_*.rs");
    cases.compile_fail("tests/ui/derive/fail_*.rs");
}
