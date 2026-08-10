//! Converts extraction requests into `sw-core` options and profiles,
//! and `sw-core` plan/report results into the extraction DTOs declared
//! by `bridge`.
//!
//! All extraction semantics — the hardware gate, collisions, topology,
//! existing-output classification and execution — live in
//! `sw_core::plan`; nothing here re-implements them.
use crate::bridge::ffi;
use std::path::PathBuf;
use sw_core::extract::{DecodeMode, ExistingOutputPolicy, ExtractOptions, ExtractReport, PathMode};
use sw_core::image::PayloadResolution;
use sw_core::mach::eval::HardwareProfile;
use sw_core::path::IrixPath;
use sw_core::plan::ExtractionPlanSummary as CorePlanSummary;

/// Builds the core extraction options and validates the output
/// directory of a request.
///
/// The output directory must be an absolute host path (the GUI always
/// produces absolute paths); it need not exist yet. The `relative_to`
/// prefix is only parsed for [`ffi::ExtractionPathMode::RelativeTo`]
/// and is validated by the authoritative [`IrixPath`] parser — the
/// bridge never re-implements the grammar.
pub(crate) fn extract_options(
    request: &ffi::ExtractionRequest,
) -> Result<(ExtractOptions, PathBuf), String> {
    if request.output_dir.is_empty() {
        return Err("output directory is empty".to_string());
    }
    let out_dir = PathBuf::from(&request.output_dir);
    if !out_dir.is_absolute() {
        return Err(format!(
            "output directory {:?} is not an absolute host path",
            request.output_dir
        ));
    }
    let path_mode = match request.path_mode {
        ffi::ExtractionPathMode::Full => PathMode::Full,
        ffi::ExtractionPathMode::Flat => PathMode::Flat,
        ffi::ExtractionPathMode::RelativeTo => {
            let prefix = IrixPath::new(&request.relative_to)
                .map_err(|error| format!("invalid relative-to prefix: {error}"))?;
            PathMode::RelativeTo(prefix)
        }
        _ => return Err("unknown extraction path mode".to_string()),
    };
    let decode = match request.decode {
        ffi::ExtractionDecodeMode::Auto => DecodeMode::Auto,
        ffi::ExtractionDecodeMode::Never => DecodeMode::Never,
        _ => return Err("unknown extraction decode mode".to_string()),
    };
    let existing_output = if request.allow_overwrite {
        ExistingOutputPolicy::Allow
    } else {
        ExistingOutputPolicy::Refuse
    };
    Ok((
        ExtractOptions {
            path_mode,
            decode,
            keep_stored: request.keep_stored,
            continue_on_error: request.continue_on_error,
            existing_output,
        },
        out_dir,
    ))
}

/// Builds the core hardware profile of a request. An empty pair list
/// means "no profile"; an empty attribute name is rejected, while an
/// empty value is a genuine fact (the media carry restrictions like
/// `GFXBOARD=`). The planner re-evaluates the selection from this
/// profile; the GUI's selection overlay is never an authority here.
pub(crate) fn hardware_profile(
    request: &ffi::ExtractionRequest,
) -> Result<Option<HardwareProfile>, String> {
    if request.hardware.is_empty() {
        return Ok(None);
    }
    let mut builder = HardwareProfile::builder();
    for pair in &request.hardware {
        if pair.attribute.is_empty() {
            return Err("hardware attribute name is empty".to_string());
        }
        builder = builder.add(&pair.attribute, &pair.value);
    }
    Ok(Some(builder.build()))
}

/// Converts a core plan summary into its bridge DTO.
pub(crate) fn plan_summary(summary: CorePlanSummary) -> ffi::ExtractionPlanSummary {
    ffi::ExtractionPlanSummary {
        requested_records: summary.requested_records as u64,
        omitted_records: summary.omitted_records as u64,
        hardware_excluded_records: summary.hardware_excluded_records as u64,
        planned_records: summary.planned_records as u64,
        output_paths: summary.output_paths as u64,
        existing_outputs: summary.existing_outputs as u64,
    }
}

/// Converts a core extraction report into its bridge DTO. Recoveries
/// stay visible: a payload found away from its expected offset is an
/// honest success, never a silent one. `Exact` resolutions are not
/// recoveries and never appear here.
pub(crate) fn report_detail(report: ExtractReport) -> ffi::ExtractionReportDetail {
    ffi::ExtractionReportDetail {
        extracted: report.extracted as u64,
        skipped: report.skipped as u64,
        failures: report
            .failures
            .into_iter()
            .map(|failure| ffi::ExtractionFailureDetail {
                path: failure.path,
                message: failure.message,
            })
            .collect(),
        recoveries: report
            .recoveries
            .into_iter()
            .filter_map(|recovery| {
                let (kind, delta) = match recovery.resolution {
                    PayloadResolution::Delta { delta } => {
                        (ffi::ExtractionRecoveryKind::Delta, Some(delta))
                    }
                    PayloadResolution::Resynced { delta } => {
                        (ffi::ExtractionRecoveryKind::Resynced, delta)
                    }
                    PayloadResolution::Scanned => (ffi::ExtractionRecoveryKind::Scanned, None),
                    // Not a recovery: exact reads are never reported.
                    PayloadResolution::Exact => return None,
                };
                Some(ffi::ExtractionRecoveryDetail {
                    path: recovery.path,
                    expected_known: recovery.expected_record_offset.is_some(),
                    expected_offset: recovery.expected_record_offset.unwrap_or(0),
                    actual_offset: recovery.actual_record_offset,
                    kind,
                    delta_known: delta.is_some(),
                    delta: delta.unwrap_or(0),
                })
            })
            .collect(),
    }
}
