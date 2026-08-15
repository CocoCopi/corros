//! Integration tests: compile a Corros program with the Corros-written
//! compiler (src/compiler.cor) and run its bytecode on the native executor,
//! exactly like the `corros` binary does, then check the output.

use corros::lexer;

/// Compile `source` with the Corros compiler and run its bytecode on the
/// native executor, returning the lines printed by `speak`.
fn run(source: &str) -> Vec<String> {
    corros::run_source(source, &[]).unwrap()
}

/// Run `source` and return the runtime error message.
fn run_err(source: &str) -> String {
    match corros::run_source(source, &[]) {
        Ok(_) => panic!("expected a runtime error"),
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------
// Basics
// ---------------------------------------------------------------------------

#[test]
fn hello_world() {
    assert_eq!(run(r#"speak("Hello, world!")"#), vec!["Hello, world!"]);
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(run("speak(2 + 3 * 4)"), vec!["14"]);
    assert_eq!(run("speak(2 ** 10)"), vec!["1024"]);
    assert_eq!(run("speak(17 % 5)"), vec!["2"]);
    assert_eq!(run("speak(-2 ** 2)"), vec!["-4"]);
}

#[test]
fn variables_and_strings() {
    assert_eq!(
        run(r#"forge name = "corros"; speak(name.loud(), name.size(), "hi " + name)"#),
        vec!["CORROS 6 hi corros"]
    );
}

#[test]
fn forge_creates_globals_at_top_level() {
    assert_eq!(run("forge x = 10; speak(x)"), vec!["10"]);
}

#[test]
fn assignment_creates_globals() {
    assert_eq!(run("x = 99; speak(x)"), vec!["99"]);
}

// ---------------------------------------------------------------------------
// Functions and closures
// ---------------------------------------------------------------------------

#[test]
fn recursion() {
    let src = r#"
        craft fib(n) {
          when n < 2 { return n }
          return fib(n - 1) + fib(n - 2)
        }
        speak(fib(15))
    "#;
    assert_eq!(run(src), vec!["610"]);
}

#[test]
fn closures_capture_and_mutate() {
    let src = r#"
        craft make_counter() {
          forge n = 0
          return craft() { n += 1; return n }
        }
        forge c = make_counter()
        speak(c(), c(), c())
    "#;
    assert_eq!(run(src), vec!["1 2 3"]);
}

#[test]
fn anonymous_crafts() {
    let src = r#"
        forge f = craft(x) { return x * 2 }
        speak(f(21))
    "#;
    assert_eq!(run(src), vec!["42"]);
}

#[test]
fn higher_order_functions() {
    let src = r#"
        craft apply(f, x) { return f(x) }
        speak(apply(craft(n) { return n + 1 }, 41))
    "#;
    assert_eq!(run(src), vec!["42"]);
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

#[test]
fn when_else() {
    let src = r#"
        forge x = 5
        when x > 3 { speak("big") } else { speak("small") }
    "#;
    assert_eq!(run(src), vec!["big"]);
}

#[test]
fn else_when_chains() {
    let src = r#"
        forge n = 9
        when n % 15 == 0 { speak("fizzbuzz") }
        else when n % 3 == 0 { speak("fizz") }
        else when n % 5 == 0 { speak("buzz") }
        else { speak(n) }
    "#;
    assert_eq!(run(src), vec!["fizz"]);
}

#[test]
fn each_loop() {
    assert_eq!(run("each i in 0..=3 { speak(i) }"), vec!["0", "1", "2", "3"]);
}

#[test]
fn each_over_list() {
    assert_eq!(
        run(r#"each x in ["a", "b"] { speak(x) }"#),
        vec!["a", "b"]
    );
}

#[test]
fn each_sum() {
    let src = "forge total = 0; each n in 1..=100 { total += n }; speak(total)";
    assert_eq!(run(src), vec!["5050"]);
}

#[test]
fn whilst_loop() {
    let src = "forge i = 0; whilst i < 3 { i += 1; speak(i) }";
    assert_eq!(run(src), vec!["1", "2", "3"]);
}

#[test]
fn break_and_onward() {
    let src = r#"
        each i in 0..10 {
          when i == 2 { onward }
          when i == 4 { break }
          speak(i)
        }
    "#;
    assert_eq!(run(src), vec!["0", "1", "3"]);
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

#[test]
fn lists() {
    let src = r#"
        forge xs = [3, 1, 2]
        xs.shove(4)
        xs.order()
        speak(xs)
        speak(xs.size(), xs[0], xs.holds(4))
    "#;
    assert_eq!(run(src), vec!["[1, 2, 3, 4]", "4 1 true"]);
}

#[test]
fn maps() {
    let src = r#"
        forge ages = { "alice": 30, "bob": 25 }
        ages["carol"] = 40
        speak(ages["alice"], ages.holds("carol"))
    "#;
    assert_eq!(run(src), vec!["30 true"]);
}

#[test]
fn ranges() {
    assert_eq!(run("speak((2..8).size(), (2..8).holds(5))"), vec!["6 true"]);
    assert_eq!(run("speak(0..5)"), vec!["0..5"]);
}

#[test]
fn string_methods() {
    let src = r#"
        forge s = "  Corros Language  "
        speak(s.shave().quiet())
        speak("a,b,c".split(","))
    "#;
    assert_eq!(run(src), vec!["corros language", "[a, b, c]"]);
}

// ---------------------------------------------------------------------------
// Operators and semantics
// ---------------------------------------------------------------------------

#[test]
fn compound_assignment() {
    let src = "forge x = 2; x += 3; x *= 4; speak(x)";
    assert_eq!(run(src), vec!["20"]);
}

#[test]
fn assignment_is_an_expression() {
    let src = "forge y = (x = 5); speak(y, x)";
    assert_eq!(run(src), vec!["5 5"]);
}

#[test]
fn indexed_assignment_is_an_expression() {
    let src = "forge xs = [1, 2, 3]; forge v = (xs[0] = 99); speak(v, xs)";
    assert_eq!(run(src), vec!["99 [99, 2, 3]"]);
}

#[test]
fn compound_indexed_assignment() {
    let src = "forge xs = [1, 2, 3]; xs[0] += 10; speak(xs)";
    assert_eq!(run(src), vec!["[11, 2, 3]"]);
}

#[test]
fn break_in_whilst_pops_body_locals() {
    let src = "craft g() { forge a = 1; whilst true { forge b = 2; break }; forge c = 3; return a + c } speak(g())";
    assert_eq!(run(src), vec!["4"]);
}

#[test]
fn onward_in_whilst_pops_body_locals() {
    let src = "craft h() { forge s = 0; forge i = 0; whilst i < 6 { forge x = 10; i = i + 1; when i % 2 == 0 { onward }; s = s + 1 }; return s } speak(h())";
    assert_eq!(run(src), vec!["3"]);
}

#[test]
fn break_and_onward_in_each_pops_body_locals() {
    let src = "craft k() { forge sum = 0; each v in [1,2,3,4] { forge d = v * 10; when d > 30 { onward }; sum = sum + v }; return sum } "
        .to_owned()
        + "craft m() { forge sum = 0; each v in [1,2,3,4,5] { forge d = v * 2; when d > 6 { break }; sum = sum + v }; return sum } "
        + "speak(k(), m())";
    assert_eq!(run(&src), vec!["6 6"]);
}

#[test]
fn assignment_creates_global_inside_expression() {
    let src = "forge xs = [1]; xs[0] = (brand_new = 7); speak(brand_new, xs)";
    assert_eq!(run(src), vec!["7 [7]"]);
}

#[test]
fn logic_operators() {
    assert_eq!(run("speak(true && false, true || false, !true)"), vec!["false true false"]);
}

#[test]
fn and_or_short_circuit() {
    let src = r#"
        craft side_effect() { speak("boom"); return true }
        speak(false && side_effect())
    "#;
    assert_eq!(run(src), vec!["false"]);
}

#[test]
fn truthiness() {
    assert_eq!(run("speak(0 == false)"), vec!["false"]);
    assert_eq!(run("speak(nil == nil)"), vec!["true"]);
}

#[test]
fn map_index_missing_key_errors() {
    let err = run_err(r#"forge m = {}; speak(m["nope"])"#);
    assert!(err.contains("no key"), "got: {err}");
}

#[test]
fn type_error_has_message() {
    let err = run_err("speak(1 + \"two\")");
    assert!(err.contains("cannot add"), "got: {err}");
}

#[test]
fn calling_non_function_errors() {
    let err = run_err("forge x = 5; x()");
    assert!(err.contains("cannot call"), "got: {err}");
}

// ---------------------------------------------------------------------------
// Lexer-level
// ---------------------------------------------------------------------------

#[test]
fn lexer_tokens() {
    let toks = lexer::lex("forge x = 1..=5 ** 2", "t.cor").unwrap();
    let kinds: Vec<_> = toks.iter().map(|t| t.kind.clone()).collect();
    use corros::lexer::TokenKind::*;
    assert_eq!(
        kinds,
        vec![
            Forge,
            Identifier("x".into()),
            Equal,
            Number(1.0),
            DotDotEqual,
            Number(5.0),
            Power,
            Number(2.0),
            Eof,
        ]
    );
}
