use std::path::Path;
use std::path::PathBuf;

pub(crate) fn validate_output_path(
    path: &str,
    allowed_extensions: &[&str],
) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("导出路径必须是绝对路径".into());
    }
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !allowed_extensions
        .iter()
        .any(|allowed| *allowed == extension)
    {
        return Err(format!("不允许导出 .{extension} 文件"));
    }
    Ok(path.to_path_buf())
}
