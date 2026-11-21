fn main() {
    slint_build::compile("ui/app.slint").expect("compile Slint UI");
    // The same artwork is uploaded as the window icon at runtime from
    // `ui/icons/app-icon-128.rgba`; this puts it into the PE resources so
    // Explorer, the task bar and shortcuts show it for the executable too.
    tftpd_r_build_support::embed_windows_icon("ui/icons/app-icon.ico")
        .expect("embed the product icon in the executable");
}
