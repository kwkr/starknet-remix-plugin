use crate::handlers::process::{do_process_command, fetch_process_result};
use crate::handlers::types::{ApiCommand, ApiCommandResult, CompileResponse};
use crate::utils::lib::{get_file_ext, get_file_path, CAIRO_COMPILERS_DIR, SIERRA_ROOT};
use crate::worker::WorkerEngine;
use rocket::fs::NamedFile;
use rocket::serde::json;
use rocket::serde::json::Json;
use rocket::tokio::fs;
use rocket::State;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::{debug, instrument};

#[instrument]
#[get("/compile-to-sierra/<version>/<remix_file_path..>")]
pub async fn compile_to_sierra(version: String, remix_file_path: PathBuf) -> Json<CompileResponse> {
    info!("/compile-to-sierra");

    let res = do_compile_to_sierra(version.clone(), remix_file_path).await;

    match res {
        Ok(res) => res,
        Err(e) => Json(CompileResponse {
            file_content: "".to_string(),
            message: e,
            status: "CompilationFailed".to_string(),
            cairo_version: version,
        }),
    }
}

#[instrument]
#[get("/compile-to-sierra-async/<version>/<remix_file_path..>")]
pub async fn compile_to_siera_async(
    version: String,
    remix_file_path: PathBuf,
    engine: &State<WorkerEngine>,
) -> String {
    info!("/compile-to-sierra-async");
    do_process_command(
        ApiCommand::SierraCompile {
            version,
            remix_file_path,
        },
        engine,
    )
}

#[instrument]
#[get("/compile-to-sierra-result/<process_id>")]
pub async fn get_siera_compile_result(process_id: String, engine: &State<WorkerEngine>) -> String {
    info!("/compile-to-sierra-result");
    fetch_process_result(process_id, engine, |result| match result {
        ApiCommandResult::SierraCompile(sierra_result) => json::to_string(&sierra_result).unwrap(),
        _ => String::from("Result not available"),
    })
}

/// Compile a given file to Sierra bytecode
///
pub async fn do_compile_to_sierra(
    version: String,
    remix_file_path: PathBuf,
) -> Result<Json<CompileResponse>, String> {
    let remix_file_path = match remix_file_path.to_str() {
        Some(path) => path.to_string(),
        None => {
            return Ok(Json(CompileResponse {
                file_content: "".to_string(),
                message: "File path not found".to_string(),
                status: "FileNotFound".to_string(),
                cairo_version: version,
            }));
        }
    };

    // check if the file has .cairo extension
    match get_file_ext(&remix_file_path) {
        ext if ext == "cairo" => {
            debug!("LOG: File extension is cairo");
        }
        _ => {
            debug!("LOG: File extension not supported");
            return Ok(Json(CompileResponse {
                file_content: "".to_string(),
                message: "File extension not supported".to_string(),
                status: "FileExtensionNotSupported".to_string(),
                cairo_version: version,
            }));
        }
    }

    let file_path = get_file_path(&remix_file_path);

    let sierra_remix_path = remix_file_path.replace(&get_file_ext(&remix_file_path), "sierra");

    let mut compile = Command::new("cargo");

    let path_to_cairo_compiler = Path::new(CAIRO_COMPILERS_DIR).join(&version);
    if path_to_cairo_compiler.exists() {
        compile.current_dir(path_to_cairo_compiler);
    } else {
        return Err(format!("Cairo compiler with version {} not found", version));
    }

    // replace .cairo with
    let sierra_path = Path::new(SIERRA_ROOT).join(&sierra_remix_path);

    // create directory for sierra file
    match sierra_path.parent() {
        Some(parent) => match fs::create_dir_all(parent).await {
            Ok(_) => {
                debug!("LOG: Created directory: {:?}", parent);
            }
            Err(e) => {
                debug!("LOG: Error creating directory: {:?}", e);
            }
        },
        None => {
            debug!("LOG: Error creating directory");
        }
    }

    let result = compile
        .arg("run")
        .arg("--release")
        .arg("--bin")
        .arg("starknet-compile")
        .arg("--")
        .arg(&file_path)
        .arg(&sierra_path)
        .arg("--single-file")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute starknet-compile: {:?}", e))?;

    debug!("LOG: ran command:{:?}", compile);

    let output = result.wait_with_output().expect("Failed to wait on child");

    Ok(Json(CompileResponse {
        file_content: match NamedFile::open(&sierra_path).await.ok() {
            Some(file) => match file.path().to_str() {
                Some(path) => match fs::read_to_string(path.to_string()).await {
                    Ok(sierra) => sierra.to_string(),
                    Err(e) => e.to_string(),
                },
                None => "".to_string(),
            },
            None => "".to_string(),
        },
        message: String::from_utf8(output.stderr)
            .unwrap()
            .replace(&file_path.to_str().unwrap().to_string(), &remix_file_path)
            .replace(
                &sierra_path.to_str().unwrap().to_string(),
                &sierra_remix_path,
            ),
        status: match output.status.code() {
            Some(0) => "Success".to_string(),
            Some(_) => "CompilationFailed".to_string(),
            None => "UnknownError".to_string(),
        },
        cairo_version: version,
    }))
}
