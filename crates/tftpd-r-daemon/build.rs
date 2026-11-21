fn main() {
    // The daemon is shipped next to the panel; both executables carry the same
    // product icon resource. The artwork lives with the GUI panel's assets
    // because that is where every exported form of it is kept.
    tftpd_r_build_support::embed_windows_icon("../tftpd-r-gui-slint/ui/icons/app-icon.ico")
        .expect("embed the product icon in the executable");
}
