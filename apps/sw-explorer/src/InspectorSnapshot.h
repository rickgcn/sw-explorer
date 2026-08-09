#pragma once

#include <QList>
#include <QMetaType>
#include <QString>
#include <QStringList>

// Qt-native mirrors of the sw::*Detail bridge structs.
//
// These are the only shapes in which the GUI thread ever sees object
// details: plain Qt value types. rust:: types exist solely inside
// BackendWorker, which performs the conversion.

// The state of a hardware-conditional installation flag. Conditional
// and unresolved flags are never collapsed into a bool.
enum class ConditionalFlagState {
    No,
    Always,
    Conditional,
    Unresolved,
};

// Hardware applicability of a product, image or subsystem.
struct HardwareSnapshot {
    // false: no descriptor record, MACH unknown. true with both lists
    // empty: known to be unrestricted. The two never conflate.
    bool known = false;
    QStringList expressions;
    QStringList unresolved;
};

// One installation flag that may be hardware-conditional.
struct ConditionalFlagSnapshot {
    ConditionalFlagState state = ConditionalFlagState::No;
    QStringList expressions;
    QStringList unresolved;
};

// The installation flags of a subsystem.
struct SubsystemFlagsSnapshot {
    // false for IDB-only subsystems: unknown, not "all clear".
    bool known = false;

    ConditionalFlagSnapshot required;
    ConditionalFlagSnapshot defaultFlag;
    ConditionalFlagSnapshot miniroot;

    bool inplace = false;
    bool patch = false;
    bool clientOnly = false;
    bool overlay = false;
    bool overlayMember = false;
};

// One versioned rule range, e.g. "patch*.sw.unix 0..1274627332".
struct RangeSnapshot {
    QString target;
    quint64 minVersion = 0;

    // When true the range is unbounded above (SGI "maxint") and
    // maxVersion carries no meaning.
    bool maxIsUnbounded = false;
    quint64 maxVersion = 0;
};

// One prereq clause: ranges that must all be satisfied together.
// Separate clauses are alternatives (OR); the structure is kept.
struct PrerequisiteClauseSnapshot {
    QList<RangeSnapshot> allOf;
};

// The dependency rules of a subsystem.
struct RulesSnapshot {
    // false for IDB-only subsystems: unknown, not "no rules".
    bool known = false;

    QList<PrerequisiteClauseSnapshot> prerequisites;
    QList<RangeSnapshot> replaces;
    QList<RangeSnapshot> incompatibilities;
    QList<RangeSnapshot> updates;
    QList<RangeSnapshot> follows;
};

// A product cut point: a filesystem tree where installation may be
// split.
struct CutpointSnapshot {
    QString path;
    quint32 sequence = 0;
};

// Everything the inspector shows for one product.
struct ProductDetailSnapshot {
    quint64 id = 0;

    QString name;
    QString title;

    bool descriptorPresent = false;
    bool idbPresent = false;

    quint64 imageCount = 0;
    quint64 subsystemCount = 0;
    quint64 entryCount = 0;

    // Meaningful only when descriptorPresent.
    QString descriptorGeneration;
    quint16 descriptorLayoutLevel = 0;
    quint32 descriptorStamp = 0;

    HardwareSnapshot mach;
    QList<CutpointSnapshot> cutpoints;
};

// Everything the inspector shows for one image.
struct ImageDetailSnapshot {
    quint64 id = 0;

    QString name;
    QString title;
    QString productName;

    bool descriptorPresent = false;
    bool idbPresent = false;

    bool versionKnown = false;
    quint64 version = 0;

    bool orderKnown = false;
    qint32 order = 0;

    quint64 subsystemCount = 0;
    quint64 entryCount = 0;

    HardwareSnapshot mach;
};

// Everything the inspector shows for one subsystem.
struct SubsystemDetailSnapshot {
    quint64 id = 0;

    QString identity;
    QString shortName;
    QString title;

    QString productName;
    QString imageName;

    bool imageVersionKnown = false;
    quint64 imageVersion = 0;

    bool descriptorPresent = false;
    bool idbPresent = false;

    quint64 entryCount = 0;

    // true exactly when the subsystem has a descriptor record; an
    // empty mapping is then "known to have no mapping".
    bool mappingKnown = false;
    QString mapping;

    HardwareSnapshot mach;
    SubsystemFlagsSnapshot flags;
    RulesSnapshot rules;

    bool autominirootKnown = false;
    QList<RangeSnapshot> autominiroot;
};

Q_DECLARE_METATYPE(ProductDetailSnapshot)
Q_DECLARE_METATYPE(ImageDetailSnapshot)
Q_DECLARE_METATYPE(SubsystemDetailSnapshot)
