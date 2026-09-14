//! Read-only bridge through production cached-index and deterministic objects APIs.
use lysilogy::{domain::PaperId, objects::from_source, source_index::load_cached};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Failure = Box<dyn std::error::Error>;
#[derive(Deserialize)]
struct Request {
    corpus_root: PathBuf,
    data_root: PathBuf,
    papers: Vec<FrozenInput>,
}
#[derive(Deserialize)]
struct FrozenInput {
    paper_id: PaperId,
    relative_path: String,
    index_sha256: String,
}
fn direct_pdf(value: &str) -> bool {
    let p = Path::new(value);
    p.components().count() == 1
        && matches!(p.components().next(), Some(Component::Normal(_)))
        && !value.starts_with('.')
        && !value.contains('\\')
        && p.extension().is_some_and(|v| v == "pdf")
}
fn safe_id(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn regular_file(path: &Path) -> Result<(), Failure> {
    for ancestor in path.ancestors() {
        if ancestor.symlink_metadata()?.file_type().is_symlink() {
            return Err("symlinked canonical input".into());
        }
    }
    if !path.symlink_metadata()?.file_type().is_file() {
        return Err("canonical input is not a regular file".into());
    }
    Ok(())
}
async fn bounded_index(path: &Path) -> Result<Vec<u8>, Failure> {
    regular_file(path)?;
    let file = tokio::fs::File::open(path).await?;
    if file.metadata().await?.len() > 32 * 1024 * 1024 {
        return Err("index exceeds 32 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("index grew beyond 32 MiB".into());
    }
    Ok(bytes)
}
async fn retain_trace(root: &Path, raw: &[u8], extension: &str) -> Result<PathBuf, Failure> {
    // A fixed external cache holds source-derived trace bytes. No canonical
    // native index or user-selected output path is ever writable here.
    for ancestor in root.ancestors() {
        match ancestor.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("symlinked trace output".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    tokio::fs::create_dir_all(root).await?;
    let path = root.join(format!("{:x}.{extension}", Sha256::digest(raw)));
    if path.exists() {
        regular_file(&path)?;
        if tokio::fs::metadata(&path).await?.len() != u64::try_from(raw.len())?
            || tokio::fs::read(&path).await? != raw
        {
            return Err("immutable trace differs".into());
        }
        return Ok(path);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let pending = root.join(format!("pending-{}-{stamp}", std::process::id()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .await?;
    file.write_all(raw).await?;
    file.sync_all().await?;
    drop(file);
    let linked = tokio::fs::hard_link(&pending, &path).await;
    tokio::fs::remove_file(&pending).await?;
    if let Err(error) = linked
        && error.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(error.into());
    }
    regular_file(&path)?;
    if tokio::fs::metadata(&path).await?.len() != u64::try_from(raw.len())?
        || tokio::fs::read(&path).await? != raw
    {
        return Err("immutable trace publication differs".into());
    }
    Ok(path)
}

// The independent Python collector implements this small framed protocol too.
// Floats here are the exact f32 values serialized from typed ReadingIndex fields.
fn native_value_digest(value: &serde_json::Value) -> Result<String, Failure> {
    fn frame(hash: &mut Sha256, tag: u8, bytes: &[u8]) -> Result<(), Failure> {
        hash.update([tag]);
        hash.update(u64::try_from(bytes.len())?.to_be_bytes());
        hash.update(bytes);
        Ok(())
    }
    #[allow(clippy::cast_possible_truncation)] // Conversion is checked exactly below.
    fn visit(value: &serde_json::Value, hash: &mut Sha256, depth: usize) -> Result<(), Failure> {
        use serde_json::Value;
        if depth > 64 {
            return Err("native basis nesting exceeds64".into());
        }
        match value {
            Value::Null => hash.update(b"n"),
            Value::Bool(value) => hash.update(if *value { b"t" } else { b"f" }),
            Value::Number(number) if number.is_i64() || number.is_u64() => {
                frame(hash, b'i', number.to_string().as_bytes())?;
            }
            Value::Number(number) => {
                let original = number.as_f64().ok_or("unsupported native number")?;
                let value = original as f32;
                if !value.is_finite() || f64::from(value).to_bits() != original.to_bits() {
                    return Err("native value is not exact finite f32".into());
                }
                hash.update(b"r");
                hash.update(value.to_bits().to_be_bytes());
            }
            Value::String(value) => frame(hash, b's', value.as_bytes())?,
            Value::Array(values) => {
                hash.update(b"a");
                hash.update(u64::try_from(values.len())?.to_be_bytes());
                for value in values {
                    visit(value, hash, depth + 1)?;
                }
            }
            Value::Object(values) => {
                hash.update(b"o");
                hash.update(u64::try_from(values.len())?.to_be_bytes());
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    frame(hash, b's', key.as_bytes())?;
                    visit(&values[key], hash, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    hash.update(b"lysilogy-native-basis-v1\0");
    visit(value, &mut hash, 0)?;
    Ok(format!("{:x}", hash.finalize()))
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let mut raw = String::new();
    io::stdin().take(1024 * 1024 + 1).read_to_string(&mut raw)?;
    if raw.len() > 1024 * 1024 {
        return Err("request exceeds 1 MiB".into());
    }
    let request: Request = serde_json::from_str(&raw)?;
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME missing")?);
    if request.corpus_root != home.join("Corpora/arxiv/pdf")
        || request.data_root != home.join(".cache/lysilogy/arxiv-kb-data")
        || request.papers.is_empty()
        || request.papers.len() > 1000
    {
        return Err("invalid measurement roots or population".into());
    }
    let mut rows = Vec::new();
    let mut output_bytes = 0;
    for paper in request.papers {
        if !safe_id(paper.paper_id.as_str()) || !direct_pdf(&paper.relative_path) {
            return Err("unsafe PDF name".into());
        }
        let directory = request
            .data_root
            .join("papers")
            .join(paper.paper_id.as_str());
        let path = directory.join("reading-index.json");
        let before = bounded_index(&path).await?;
        regular_file(&request.corpus_root.join(&paper.relative_path))?;
        if format!("{:x}", Sha256::digest(&before)) != paper.index_sha256 {
            return Err("index receipt differs".into());
        }
        let document = load_cached(&request.corpus_root.join(&paper.relative_path), &directory)
            .await?
            .ok_or("frozen index is not a valid production cache")?;
        if document.etag != format!("\"{}\"", paper.index_sha256)
            || bounded_index(&path).await? != before
        {
            return Err("index changed while loading".into());
        }
        // Commit the exact typed input, retaining no full native text in the
        // transport. The canonical whole-index hash remains independently bound.
        let mut native_basis = serde_json::to_value(&document.index)?;
        native_basis
            .as_object_mut()
            .ok_or("native basis is not an object")?
            .remove("figures");
        let native_basis_sha256 = native_value_digest(&native_basis)?;
        let derived = from_source(
            &request.corpus_root.join(&paper.relative_path),
            &paper.paper_id,
            &document,
        )
        .await?;
        let artifact = derived.artifact;
        let graphics = artifact
            .graphics
            .as_ref()
            .ok_or("source factory omitted graphics status")?;
        let graphics_basis_json = String::from_utf8(graphics.basis_json()?)?;
        let mut graphics_traces = Vec::new();
        for (page, raw) in derived.traces {
            let path = retain_trace(
                &home.join(".cache/lysilogy/object-graphics-traces"),
                &raw,
                "xml",
            )
            .await?;
            graphics_traces.push(
                json!({"page":page,"sha256":format!("{:x}",Sha256::digest(&raw)),"path":path}),
            );
        }
        let mut graphics_masks = Vec::new();
        for (page, raw) in derived.mask_receipts {
            let path = retain_trace(
                &home.join(".cache/lysilogy/object-graphics-masks"),
                &raw,
                "json",
            )
            .await?;
            graphics_masks.push(
                json!({"page":page,"sha256":format!("{:x}",Sha256::digest(&raw)),"path":path}),
            );
        }
        let mut graphics_vectors = Vec::new();
        for (page, raw) in derived.vector_rasters {
            let path = retain_trace(
                &home.join(".cache/lysilogy/object-graphics-vectors"),
                &raw,
                "pam",
            )
            .await?;
            graphics_vectors.push(
                json!({"page":page,"sha256":format!("{:x}",Sha256::digest(&raw)),"path":path}),
            );
        }
        if bounded_index(&path).await? != before {
            return Err("canonical index changed during graphics derivation".into());
        }
        let artifact_json = serde_json::to_string(&artifact)?;
        let row = json!({"paper_id":paper.paper_id,"index_sha256":paper.index_sha256,"object_sha256":format!("{:x}",Sha256::digest(artifact_json.as_bytes())),"artifact_json":artifact_json,"native_basis_sha256":native_basis_sha256,"native_basis_format":"native-json-f32-v1","native_schema_version":document.index.schema_version,"graphics_basis_json":graphics_basis_json,"graphics_traces":graphics_traces,"graphics_masks":graphics_masks,"graphics_vectors":graphics_vectors});
        output_bytes += serde_json::to_vec(&row)?.len() + 1;
        if output_bytes > 16 * 1024 * 1024 - 1024 {
            return Err("object response exceeds16MiB".into());
        }
        rows.push(row);
    }
    println!(
        "{}",
        json!({"schema_version":1,"papers":rows,"network_calls":0,"model_calls":0})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn should_match_independent_protocol_vectors_when_values_have_exact_native_types() {
        fn expand(value: &serde_json::Value) -> serde_json::Value {
            if let Some(bits) = value
                .as_object()
                .filter(|v| v.len() == 1)
                .and_then(|v| v.get("$f32_bits"))
            {
                let bits = u32::from_str_radix(bits.as_str().unwrap(), 16).unwrap();
                return serde_json::to_value(f32::from_bits(bits)).unwrap();
            }
            match value {
                serde_json::Value::Array(values) => values.iter().map(expand).collect(),
                serde_json::Value::Object(values) => values
                    .iter()
                    .map(|(key, value)| (key.clone(), expand(value)))
                    .collect(),
                _ => value.clone(),
            }
        }
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../eval/native-basis-vectors.json")).unwrap();
        for row in vectors["vectors"].as_array().unwrap() {
            assert_eq!(
                native_value_digest(&expand(&row["value"])).unwrap(),
                row["sha256"].as_str().unwrap(),
                "{}",
                row["name"]
            );
        }
        assert!(native_value_digest(&json!(0.1234567890123_f64)).is_err());
        assert!(native_value_digest(&json!(1e300_f64)).is_err());
    }

    #[test]
    fn should_reject_unvalidated_ids_when_json_deserialization_bypasses_from_str() {
        for value in [
            "../outside",
            "/outside",
            "ABCDEF1234567890",
            "a",
            "aaaaaaaaaaaaaa/.",
        ] {
            assert!(!safe_id(value));
        }
        assert!(safe_id("0123456789abcdef"));
    }
    #[tokio::test]
    async fn should_reject_oversized_or_redirected_inputs_when_bridge_reads_directly() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("index.json");
        let handle = std::fs::File::create(&file).unwrap();
        handle.set_len(32 * 1024 * 1024 + 1).unwrap();
        assert!(bounded_index(&file).await.is_err());
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(bounded_index(&link).await.is_err());
    }
    #[test]
    fn should_reject_unsafe_names_when_bridge_reads_frozen_pdfs() {
        for value in [
            "../x.pdf", "/x.pdf", "a/x.pdf", "a\\x.pdf", ".x.pdf", "x.txt",
        ] {
            assert!(!direct_pdf(value));
        }
        assert!(direct_pdf("2104.01511v1.pdf"));
    }
}
