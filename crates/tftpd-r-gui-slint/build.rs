fn main() {
    // Bundle the `.po` catalogues into the executable. English is the source
    // language of the `@tr(...)` strings, so `translations/` only has to hold
    // the languages that are actually translated (currently zh_CN). The default
    // translation context is disabled here and in the catalogue (no msgctxt):
    // msgids are the plain source strings.
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("translations")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile Slint UI");
    // A changed catalogue must rebuild the UI; without this, editing a `.po`
    // file can go unnoticed until an unrelated `.slint` change triggers a
    // rebuild (slint-ui/slint#13321).
    println!("cargo:rerun-if-changed=translations");
    // The same artwork is uploaded as the window icon at runtime from
    // `ui/icons/app-icon-128.rgba`; this puts it into the PE resources so
    // Explorer, the task bar and shortcuts show it for the executable too.
    tftpd_r_build_support::embed_windows_icon("ui/icons/app-icon.ico")
        .expect("embed the product icon in the executable");
}
