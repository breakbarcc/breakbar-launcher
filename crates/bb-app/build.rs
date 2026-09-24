fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("failed to compile Slint UI");

    embed_resource::compile("assets/breakbar.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed Windows resources");
}
