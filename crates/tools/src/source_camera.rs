//! Source camera inspection (donor `tools/source-camera.ts`).
//!
//! Normalizes and samples `.camera` authoring files without starting a
//! renderer, using the `qa-client` camera grammar and playback.

use std::path::Path;

use qa_client::camera::{parse_camera, serialize_camera, CameraPlayback, CameraSample};

use crate::error::ToolsError;
use crate::fsutil::{read_text, write_bytes};
use crate::json::Json;

/// Inspect, normalize, and sample authoring files.
pub fn camera_tool(args: &[String]) -> Result<(), ToolsError> {
    let (command, input, output, interval) = match args {
        [command, input, output] => (command.as_str(), input.as_str(), output.as_str(), None),
        [command, input, output, interval] => (command.as_str(), input.as_str(), output.as_str(), Some(interval.as_str())),
        _ => return Err(usage()),
    };
    if (command != "normalize" && command != "sample") || args.len() > 4 {
        return Err(usage());
    }
    let definition = parse_camera(&read_text(Path::new(input))?)?;
    if command == "normalize" {
        write_bytes(Path::new(output), serialize_camera(&definition)?.as_bytes())?;
        return Ok(());
    }
    let step: f64 = match interval {
        None => 16.0,
        Some(text) => text.parse().unwrap_or(f64::NAN),
    };
    if !step.is_finite() || step <= 0.0 {
        return Err(ToolsError::invalid("Camera sample interval must be positive"));
    }
    let mut playback = CameraPlayback::new(definition, 0.0)?;
    let mut rows = Vec::new();
    let mut time = 0.0;
    loop {
        match playback.sample(time)? {
            None => break,
            Some(sample) => rows.push(sample_row(time, &sample)),
        }
        time += step;
    }
    write_bytes(Path::new(output), format!("{}\n", Json::array(rows).render_pretty()).as_bytes())?;
    Ok(())
}

fn usage() -> ToolsError {
    ToolsError::invalid("Usage: source-camera normalize <input.camera> <output.camera> | sample <input.camera> <output.json> [step-ms]")
}

fn vec_json(x: f32, y: f32, z: f32) -> Json {
    Json::object(vec![
        ("x".to_owned(), Json::float(f64::from(x))),
        ("y".to_owned(), Json::float(f64::from(y))),
        ("z".to_owned(), Json::float(f64::from(z))),
    ])
}

fn sample_row(milliseconds: f64, sample: &CameraSample) -> Json {
    Json::object(vec![
        ("milliseconds".to_owned(), Json::float(milliseconds)),
        ("origin".to_owned(), vec_json(sample.origin.x, sample.origin.y, sample.origin.z)),
        ("direction".to_owned(), vec_json(sample.direction.x, sample.direction.y, sample.direction.z)),
        ("fov".to_owned(), Json::float(f64::from(sample.fov))),
        (
            "events".to_owned(),
            Json::array(
                sample
                    .events
                    .iter()
                    .map(|event| {
                        Json::object(vec![
                            ("type".to_owned(), Json::int(i64::from(event.event_type))),
                            ("param".to_owned(), Json::string(&event.param)),
                            ("time".to_owned(), Json::float(event.time)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}
