use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/windows/app.manifest");
    println!("cargo:rerun-if-changed=assets/icons/sizes/16.png");
    println!("cargo:rerun-if-changed=assets/icons/sizes/24.png");
    println!("cargo:rerun-if-changed=assets/icons/sizes/32.png");
    println!("cargo:rerun-if-changed=assets/icons/sizes/48.png");
    println!("cargo:rerun-if-changed=assets/icons/sizes/64.png");
    println!("cargo:rerun-if-changed=assets/icons/sizes/128.png");
    println!("cargo:rerun-if-changed=assets/icons/sizes/256.png");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let ico_path = out_dir.join("wizrust101-obs.ico");
    write_ico(&manifest_dir, &ico_path).expect("generate Windows application icon");

    let rc_path = out_dir.join("wizrust101-obs.rc");
    let res_path = out_dir.join("wizrust101-obs.res");
    let manifest = manifest_dir.join("assets/windows/app.manifest");
    let rc = format!(
        "1 ICON \"{}\"\n1 24 \"{}\"\n",
        rc_quote(&ico_path),
        rc_quote(&manifest)
    );
    fs::write(&rc_path, rc).expect("write Windows resources file");

    let rc_exe = locate_rc().expect("Windows SDK resource compiler (rc.exe) is required");
    let result = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", rc_exe.display()));
    assert!(result.success(), "rc.exe failed to compile app resources");
    println!(
        "cargo:rustc-link-arg-bin=wizrust101-obs={}",
        res_path.display()
    );
}

fn write_ico(manifest_dir: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    let sizes = [16_u16, 24, 32, 48, 64, 128, 256];
    let mut frames = Vec::with_capacity(sizes.len());
    for size in sizes {
        frames.push(fs::read(
            manifest_dir
                .join("assets/icons/sizes")
                .join(format!("{size}.png")),
        )?);
    }
    let directory_size = 6 + frames.len() * 16;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    let mut offset = directory_size as u32;
    for (size, frame) in sizes.into_iter().zip(&frames) {
        bytes.push(if size == 256 { 0 } else { size as u8 });
        bytes.push(if size == 256 { 0 } else { size as u8 });
        bytes.push(0);
        bytes.push(0);
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&32_u16.to_le_bytes());
        bytes.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        offset += frame.len() as u32;
    }
    for frame in frames {
        bytes.extend_from_slice(&frame);
    }
    fs::write(path, bytes)
}

fn rc_quote(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn locate_rc() -> Option<PathBuf> {
    if let Some(path) = env::var_os("RC") {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = find_on_path("rc.exe") {
        return Some(path);
    }
    let root = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)?
        .join("Windows Kits/10/bin");
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        _ => "x86",
    };
    let mut versions: Vec<_> = fs::read_dir(root).ok()?.filter_map(Result::ok).collect();
    versions.sort_by_key(|entry| std::cmp::Reverse(entry.file_name()));
    versions
        .into_iter()
        .map(|entry| entry.path().join(architecture).join("rc.exe"))
        .find(|path| path.is_file())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}
