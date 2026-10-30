//! Compiles the Slint UI (Fluent style), draws the app icon and embeds it,
//! the version info and the manifest (per-monitor DPI awareness) into the
//! .exe.

#[path = "src/art.rs"]
mod art;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="HandyShortcut" type="win32"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges>
      <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
    </requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
</assembly>
"#;

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("compiling ui/app.slint");

    println!("cargo:rerun-if-changed=src/art.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("app.ico");
    std::fs::write(&ico, art::ico(&[16, 20, 24, 32, 40, 48, 64, 96, 128, 256], art::APP)).unwrap();

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap())
        .set_manifest(MANIFEST)
        .set("FileDescription", "Handy Shortcut")
        .set("ProductName", "Handy Shortcut");
    res.compile().expect("embedding Windows resources");
}
