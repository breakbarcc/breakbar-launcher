use super::accounts::file_name_safe;

#[test]
fn shortcut_file_names_drop_forbidden_characters() {
    assert_eq!(file_name_safe("Main"), "Main");
    assert_eq!(
        file_name_safe(r#"a<b>c:d"e/f\g|h?i*j"#),
        "a_b_c_d_e_f_g_h_i_j"
    );
    // Windows drops trailing dots and spaces from file names.
    assert_eq!(file_name_safe("Alt. "), "Alt");
}
