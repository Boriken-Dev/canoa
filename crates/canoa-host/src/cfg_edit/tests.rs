use super::*;

fn table(text: &str) -> toml::Table {
    toml::from_str(text).expect("a table")
}

fn edited(text: &str, edit: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> String {
    let mut document = parse(text).expect("TOML");
    edit(&mut document).expect("edited");
    let out = document.to_string();
    // What was written is TOML that says what was meant.
    out.parse::<DocumentMut>().expect("the result is TOML");
    out
}

const PERSON: &str = "\
# my canoa.cfg
[appearance]
brightness = \"dark\"   # night owl
look = \"canoa\"

# who may connect
[host.agents.a] # the first
harness = \"claude\"
accounts = [\"x\"]

[usage]
warn_permille = 800
";

#[test]
fn a_replaced_table_changes_its_keys_and_leaves_every_other_byte() {
    let out = edited(PERSON, |document| {
        replace(
            document,
            &["appearance"],
            &table("brightness = \"light\"\nlook = \"canoa\"\n"),
        )
    });
    assert_eq!(
        out,
        PERSON.replace("brightness = \"dark\"   #", "brightness = \"light\"   #")
    );
}

#[test]
fn a_key_the_new_table_lacks_goes_and_one_it_has_is_added_where_the_table_is() {
    let out = edited(PERSON, |document| {
        replace(
            document,
            &["appearance"],
            &table("look = \"canoa\"\ndensity = \"verbose\"\n"),
        )
    });
    assert!(!out.contains("brightness"), "{out}");
    assert!(out.contains("density = \"verbose\""), "{out}");
    assert!(out.contains("# my canoa.cfg"), "{out}");
    assert!(
        out.contains("# who may connect\n[host.agents.a] # the first"),
        "{out}"
    );
    assert!(out.contains("[usage]\nwarn_permille = 800\n"), "{out}");
}

#[test]
fn a_table_the_file_lacks_is_added_after_the_others_whatever_the_file_ends_with() {
    for text in [PERSON, "[a]\nx = 1", "", "# only a comment\n"] {
        let out = edited(text, |document| {
            replace(document, &["new"], &table("k = 1\n"))
        });
        assert!(out.starts_with(text), "{out}");
        let read = table(&out);
        assert_eq!(read["new"]["k"].as_integer(), Some(1), "{out}");
        assert!(out.trim_end().ends_with("k = 1"), "{out}");
    }
}

#[test]
fn a_table_made_under_tables_the_file_has_is_a_header_of_its_own_beside_them() {
    let out = edited(PERSON, |document| {
        merge(
            document,
            &["host", "pools", "p"],
            &table("standard = \"s\"\naccounts = [\"a\", \"b\"]\n"),
        )
    });
    assert!(
        out.contains("[host.pools.p]\nstandard = \"s\"\naccounts = [\"a\", \"b\"]\n"),
        "{out}"
    );
    assert!(out.starts_with(PERSON.trim_end()), "{out}");
}

#[test]
fn a_name_is_quoted_only_when_it_has_to_be_and_found_again_as_written() {
    for name in ["plain-1_x", "a b", "a.b", "café", "naïve \"one\"", "日本"] {
        let out = edited("[host]\n", |document| {
            merge(
                document,
                &["host", "accounts", name],
                &table("harness = \"claude\"\n"),
            )
        });
        let read = table(&out);
        assert_eq!(
            read["host"]["accounts"][name]["harness"].as_str(),
            Some("claude"),
            "{name}: {out}"
        );
        let again = edited(&out, |document| {
            merge(
                document,
                &["host", "accounts", name],
                &table("kind = \"k\"\n"),
            )
        });
        let read = table(&again);
        assert_eq!(
            read["host"]["accounts"].as_table().map(toml::Table::len),
            Some(1),
            "{name}: {again}"
        );
        assert_eq!(
            read["host"]["accounts"][name]["kind"].as_str(),
            Some("k"),
            "{name}: {again}"
        );
    }
    let plain = edited("", |document| {
        merge(document, &["host", "accounts", "work"], &table("k = 1\n"))
    });
    assert!(plain.contains("[host.accounts.work]"), "{plain}");
}

#[test]
fn a_header_with_a_comment_or_spaces_is_found_and_edited_not_added_again() {
    let text = "[ host . agents . a ]   # mine\nharness = \"claude\"\n";
    let out = edited(text, |document| {
        merge(
            document,
            &["host", "agents", "a"],
            &table("accounts = [\"x\"]\n"),
        )
    });
    assert!(out.contains("# mine"), "{out}");
    assert_eq!(out.matches("harness").count(), 1, "{out}");
    let read = table(&out);
    assert_eq!(
        read["host"]["agents"]["a"]["accounts"][0].as_str(),
        Some("x"),
        "{out}"
    );
    assert_eq!(
        read["host"]["agents"]["a"]["harness"].as_str(),
        Some("claude"),
        "{out}"
    );
}

#[test]
fn removing_a_table_takes_the_blank_line_before_it_and_nothing_else() {
    let out = edited(PERSON, |document| {
        assert!(remove(document, &["host", "agents", "a"])?);
        Ok(())
    });
    assert_eq!(
        out,
        "# my canoa.cfg\n[appearance]\nbrightness = \"dark\"   # night owl\nlook = \"canoa\"\n\n[usage]\nwarn_permille = 800\n"
    );
    let mut document = parse(PERSON).expect("TOML");
    assert!(!remove(&mut document, &["host", "agents", "none"]).expect("a path"));
    assert!(!remove(&mut document, &["nothing", "here"]).expect("a path"));
    assert_eq!(document.to_string(), PERSON);
}

#[test]
fn a_key_is_taken_out_and_a_table_inside_one_is_seen() {
    let mut document = parse(PERSON).expect("TOML");
    assert!(remove_key(&mut document, &["host", "agents", "a"], "accounts").expect("removed"));
    assert!(!remove_key(&mut document, &["host", "agents", "a"], "accounts").expect("absent"));
    assert!(!document.to_string().contains("accounts"));
    assert!(!has_subtable(&document, &["host", "agents", "a"]));
    assert!(has_subtable(&document, &["host", "agents"]));
    let nested = parse("[a]\nx = 1\n[a.b]\ny = 2\n").expect("TOML");
    assert!(has_subtable(&nested, &["a"]));
    assert!(!has_subtable(&nested, &["a", "b"]));
}

#[test]
fn a_value_where_a_table_is_wanted_is_refused_by_name() {
    let mut document = parse("host = 3\n").expect("TOML");
    let error =
        merge(&mut document, &["host", "agents", "a"], &table("k = 1\n")).expect_err("refused");
    assert!(error.contains("host"), "{error}");
}

#[test]
fn nested_tables_arrays_and_numbers_are_written_as_the_value_they_are() {
    let value = table(
        "kind = \"k\"\nn = 3\nf = 1.5\nok = true\nlist = [1, 2]\nmixed = [{ a = 1 }]\n[settings]\ndeep = \"yes\"\n[settings.more]\nx = 1\n",
    );
    let out = edited("", |document| replace(document, &["secrets"], &value));
    assert_eq!(table(&out)["secrets"], toml::Value::Table(value), "{out}");
}

#[test]
fn what_is_equal_is_left_as_the_person_spelled_it() {
    let text = "[t]\nbig = 0x10   # hex\nname = 'single'\nlist = [ 1,2 ]\n";
    let out = edited(text, |document| {
        replace(
            document,
            &["t"],
            &table("big = 16\nname = \"single\"\nlist = [1, 2]\n"),
        )
    });
    assert_eq!(out, text);
}
