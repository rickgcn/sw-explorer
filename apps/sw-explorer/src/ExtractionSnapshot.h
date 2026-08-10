#pragma once

#include "EntrySnapshot.h"
#include "HardwareSnapshot.h"

#include <QList>
#include <QMetaType>
#include <QString>

// Qt-native mirrors of the sw::Extraction* bridge structs.
//
// These are the only shapes in which the GUI thread ever sees
// extraction requests, plans and reports: plain Qt value types.
// rust:: types exist solely inside BackendWorker, which performs the
// conversion.
//
// An extraction scope is a concrete list of entry keys — exactly the
// rows the user is looking at — never a search text or a hierarchy
// object id that would be re-interpreted.

// How entry paths are mapped below the output directory.
enum class ExtractionPathMode {
    Full,
    Flat,
    RelativeTo,
};

// Whether compressed payloads are decoded during extraction.
enum class ExtractionDecodeMode {
    Auto,
    Never,
};

// Everything one extraction needs, for both preflight planning and
// execution. The hardware profile is the applied profile verbatim;
// the planner re-evaluates the selection from it, so the on-screen
// selection overlay is never an extraction authority.
struct ExtractionRequestSnapshot {
    // The entries to extract, in display order.
    QList<EntryKey> entries;
    // The hardware profile to apply; empty means no profile.
    HardwareProfileSnapshot hardware;
    // The output directory; must be absolute, need not exist yet.
    QString outputDir;
    ExtractionPathMode pathMode = ExtractionPathMode::Full;
    // The prefix for RelativeTo; ignored by the other modes.
    QString relativeTo;
    ExtractionDecodeMode decode = ExtractionDecodeMode::Auto;
    bool keepStored = false;
    bool continueOnError = true;
    // Allow overwriting existing regular files; symbolic links,
    // directories and special files are never overwrite targets.
    bool allowOverwrite = false;
};

// Counts describing a confirmed extraction plan, for user
// confirmation. The planned entries themselves never cross back; the
// actual extraction re-plans from the same request.
struct ExtractionPlanSnapshot {
    quint64 requestedRecords = 0;
    quint64 omittedRecords = 0;
    quint64 hardwareExcludedRecords = 0;
    quint64 plannedRecords = 0;
    quint64 outputPaths = 0;
    quint64 existingOutputs = 0;
};

// A single entry that failed to extract.
struct ExtractionFailureSnapshot {
    QString path;
    QString message;
};

// How a payload record was located when it was not at its expected
// offset.
enum class ExtractionRecoveryKind {
    Delta,
    Resynced,
    Scanned,
};

// A payload that was not found exactly at its expected offset: an
// honest success the user must see, not a silent one.
struct ExtractionRecoverySnapshot {
    QString path;
    bool expectedKnown = false;
    quint64 expectedOffset = 0;
    quint64 actualOffset = 0;
    ExtractionRecoveryKind kind = ExtractionRecoveryKind::Delta;
    bool deltaKnown = false;
    qint64 delta = 0;
};

// The outcome of an executed extraction. Individual failures are
// data, not dialog errors: a planner refusal (zero writes) travels
// the failed signal instead.
struct ExtractionReportSnapshot {
    quint64 extracted = 0;
    quint64 skipped = 0;
    QList<ExtractionFailureSnapshot> failures;
    QList<ExtractionRecoverySnapshot> recoveries;
};

Q_DECLARE_METATYPE(ExtractionRequestSnapshot)
Q_DECLARE_METATYPE(ExtractionPlanSnapshot)
Q_DECLARE_METATYPE(ExtractionReportSnapshot)
