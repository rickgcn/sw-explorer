#pragma once

#include <QList>
#include <QMetaType>
#include <QString>
#include <QStringList>

#include "EntrySnapshot.h"

// Qt-native mirrors of the sw::Hardware* / sw::Selection* bridge
// structs.
//
// A hardware profile is a list of facts about the simulated target
// machine (attribute/value pairs), never a MACH expression; the same
// attribute may appear several times with different values. A
// selection snapshot carries entry identities only — which IDB
// records a profile selects and which paths conflict — and is purely
// an overlay over the entry list, never a replacement for it.
//
// rust:: types exist solely inside BackendWorker, which performs the
// conversion.

// One hardware attribute/value pair, e.g. CPUBOARD=IP22. Unknown
// attribute names and values pass through unchanged.
struct HardwareValueSnapshot {
    QString attribute;
    QString value;
};

using HardwareProfileSnapshot = QList<HardwareValueSnapshot>;

// The candidate values of one hardware attribute, discovered in the
// committed distribution's parsed MACH expressions.
struct HardwareCandidateSetSnapshot {
    QString attribute;
    QStringList values;
};

using HardwareCandidatesSnapshot = QList<HardwareCandidateSetSnapshot>;

// One contested path plus the entries competing for it, exactly as
// the core selection reports it.
struct SelectionConflictSnapshot {
    QString path;
    QList<EntryKey> candidates;
};

// Which entries the current hardware profile selects across the
// whole distribution. Conflict candidates stay included in
// `selected`; nothing is resolved, picked or dropped on the Qt side.
struct SelectionSnapshot {
    QList<EntryKey> selected;
    QList<SelectionConflictSnapshot> conflicts;
};

Q_DECLARE_METATYPE(HardwareValueSnapshot)
Q_DECLARE_METATYPE(HardwareProfileSnapshot)
Q_DECLARE_METATYPE(HardwareCandidateSetSnapshot)
Q_DECLARE_METATYPE(HardwareCandidatesSnapshot)
Q_DECLARE_METATYPE(SelectionConflictSnapshot)
Q_DECLARE_METATYPE(SelectionSnapshot)
