#pragma once

#include <QList>
#include <QMetaType>
#include <QString>
#include <QStringList>

// Qt-native mirrors of the sw::Entry* bridge structs.
//
// These are the only shapes in which the GUI thread ever sees IDB
// entries: plain Qt value types. rust:: types exist solely inside
// BackendWorker, which performs the conversion.
//
// An entry row is an IDB record, not a deduplicated installed
// filesystem: the same path may appear several times and every
// occurrence is its own row.

// The kind of filesystem object an IDB record describes. A type
// letter the bridge does not know maps to Other, with the raw letter
// preserved in fileTypeRaw.
enum class EntryFileType {
    Regular,
    Directory,
    SymbolicLink,
    BlockDevice,
    CharacterDevice,
    Fifo,
    Other,
};

// Backend identity of one entry row: the product's hierarchy object
// id plus the sw-core entry id inside that product. EntryId 0 is a
// perfectly valid first entry; it is never an error marker.
struct EntryKey {
    quint64 productId = 0;
    quint64 entryId = 0;
};

inline bool operator==(const EntryKey &lhs, const EntryKey &rhs)
{
    return lhs.productId == rhs.productId && lhs.entryId == rhs.entryId;
}

inline bool operator!=(const EntryKey &lhs, const EntryKey &rhs)
{
    return !(lhs == rhs);
}

inline size_t qHash(const EntryKey &key, size_t seed = 0) noexcept
{
    return qHashMulti(seed, key.productId, key.entryId);
}

// One row of the entry browser: one IDB record.
struct EntrySummarySnapshot {
    // Object id of the containing product.
    quint64 productId = 0;
    // The sw-core entry id: the 0-based position in the product's
    // entry list. Zero is a perfectly valid id.
    quint64 entryId = 0;

    QString path;
    QString subsystem;

    EntryFileType fileType = EntryFileType::Regular;
    // The raw IDB type letter; meaningful even for Other.
    QString fileTypeRaw;

    bool sizeKnown = false;
    quint64 size = 0;

    // Bytes occupied in the image archive; a cmpsize(0) record never
    // surfaces as "stored size 0".
    bool storedSizeKnown = false;
    quint64 storedSize = 0;

    QStringList mach;
    // Raw mach payloads that could not be parsed; never dropped.
    QStringList unresolvedMach;
};

using EntryListSnapshot = QList<EntrySummarySnapshot>;

// Everything the inspector shows for one IDB entry.
struct EntryDetailSnapshot {
    quint64 productId = 0;
    quint64 entryId = 0;

    EntryFileType fileType = EntryFileType::Regular;
    QString fileTypeRaw;

    quint32 mode = 0;
    QString owner;
    QString group;

    QString path;
    QString rawPath;
    QString sourcePath;
    QString subsystem;

    bool sizeKnown = false;
    quint64 size = 0;

    // cmpsize as stored in the archive; zero means stored
    // uncompressed.
    bool compressedSizeKnown = false;
    quint64 compressedSize = 0;

    bool storedSizeKnown = false;
    quint64 storedSize = 0;

    bool checksumKnown = false;
    quint64 checksum = 0;

    // suggest / update / noupdate, or the raw value of an unknown
    // mode.
    bool configKnown = false;
    QString configMode;

    bool symlinkTargetKnown = false;
    QString symlinkTarget;

    bool deviceKnown = false;
    quint32 deviceMajor = 0;
    quint32 deviceMinor = 0;

    // An entry without MACH attributes is known to be unrestricted,
    // which is not the same as unknown.
    QStringList mach;
    QStringList unresolvedMach;

    // Payload metadata only; nothing was read from the archive.
    bool payloadPresent = false;
    QString payloadImage;

    bool payloadEncodedSizeKnown = false;
    quint64 payloadEncodedSize = 0;

    // Expected (unverified) record offset from the layout algorithm;
    // never an actual, read-confirmed offset.
    bool expectedRecordOffsetKnown = false;
    quint64 expectedRecordOffset = 0;

    QString originIdbPath;
    quint64 originLine = 0;
    QString rawIdbLine;
};

Q_DECLARE_METATYPE(EntryFileType)
Q_DECLARE_METATYPE(EntryKey)
Q_DECLARE_METATYPE(EntrySummarySnapshot)
Q_DECLARE_METATYPE(EntryListSnapshot)
Q_DECLARE_METATYPE(EntryDetailSnapshot)
