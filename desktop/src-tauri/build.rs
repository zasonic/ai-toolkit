fn main() {
    #[cfg(windows)]
    {
        // Embed an app manifest that opts the launcher into long-path support
        // (so the bundle still works when unzipped to a deeply nested folder)
        // and UTF-8 active code page.
        use embed_manifest::{embed_manifest_file, new_manifest};
        let _ = new_manifest; // keep the import alive even if the call below fails
        if let Err(e) = embed_manifest_file("longpath.manifest") {
            println!("cargo:warning=failed to embed Windows manifest: {e}");
        }
        println!("cargo:rerun-if-changed=longpath.manifest");
    }

    tauri_build::build()
}
