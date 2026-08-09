#include "EntryTableModel.h"

#include "ByteFormat.h"

#include <QSet>
#include <QStringList>

EntryTableModel::EntryTableModel(QObject *parent)
    : QAbstractTableModel(parent)
{
}

bool EntryTableModel::setEntries(const EntryListSnapshot &snapshot, QString *errorMessage)
{
    const auto fail = [errorMessage](const QString &message) {
        if (errorMessage != nullptr) {
            *errorMessage = message;
        }
        return false;
    };

    // Validate the whole snapshot before anything is replaced: entry
    // keys must be unique. Duplicate paths are perfectly legal (IDB
    // records, not a unique filesystem) and are never rejected.
    //
    // The key is the full (productId, entryId) pair, never a
    // hand-packed u64: both ids are 64-bit, and folding them into
    // one value would make distinct keys collidable in principle.
    QSet<QPair<quint64, quint64>> seenKeys;
    seenKeys.reserve(snapshot.size());
    for (const EntrySummarySnapshot &entry : snapshot) {
        const QPair<quint64, quint64> key{entry.productId, entry.entryId};
        if (seenKeys.contains(key)) {
            return fail(tr("duplicate entry key (product %1, entry %2)")
                            .arg(entry.productId)
                            .arg(entry.entryId));
        }
        seenKeys.insert(key);
    }

    beginResetModel();
    m_entries = snapshot;
    endResetModel();
    return true;
}

EntryKey EntryTableModel::entryKey(const QModelIndex &index) const
{
    if (!index.isValid() || index.model() != this || index.row() >= m_entries.size()) {
        return {};
    }
    const EntrySummarySnapshot &entry = m_entries.at(index.row());
    return {entry.productId, entry.entryId};
}

int EntryTableModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : static_cast<int>(m_entries.size());
}

int EntryTableModel::columnCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : ColumnCount;
}

QVariant EntryTableModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.model() != this || index.row() >= m_entries.size()) {
        return {};
    }
    const EntrySummarySnapshot &entry = m_entries.at(index.row());

    switch (role) {
    case Qt::DisplayRole:
        switch (index.column()) {
        case PathColumn:
            return entry.path;
        case TypeColumn:
            return typeText(entry);
        case SizeColumn:
            // Unknown is a dash, never a fake zero.
            return entry.sizeKnown ? formatByteSize(entry.size) : QStringLiteral("—");
        case StoredColumn:
            return entry.storedSizeKnown ? formatByteSize(entry.storedSize)
                                         : QStringLiteral("—");
        case MachColumn: {
            QStringList parts = entry.mach;
            if (!entry.unresolvedMach.isEmpty()) {
                // Unresolved MACH must never look like "no MACH".
                parts.append(tr("⚠ unresolved"));
            }
            return parts.join(QStringLiteral("; "));
        }
        case SubsystemColumn:
            return entry.subsystem;
        default:
            return {};
        }
    case Qt::ToolTipRole:
        switch (index.column()) {
        case SizeColumn:
            return entry.sizeKnown ? tr("%1 bytes").arg(entry.size) : QVariant();
        case StoredColumn:
            return entry.storedSizeKnown ? tr("%1 bytes").arg(entry.storedSize) : QVariant();
        case MachColumn:
            return machToolTip(entry);
        case TypeColumn:
            if (entry.fileType == EntryFileType::Other) {
                return tr("Unknown entry type '%1'").arg(entry.fileTypeRaw);
            }
            return {};
        default:
            return {};
        }
    case Qt::TextAlignmentRole:
        if (index.column() == SizeColumn || index.column() == StoredColumn) {
            return QVariant::fromValue(Qt::AlignRight | Qt::AlignVCenter);
        }
        return {};
    case SizeBytesRole:
        return entry.sizeKnown ? QVariant::fromValue(entry.size) : QVariant();
    case StoredBytesRole:
        return entry.storedSizeKnown ? QVariant::fromValue(entry.storedSize) : QVariant();
    default:
        return {};
    }
}

QVariant EntryTableModel::headerData(int section, Qt::Orientation orientation, int role) const
{
    if (orientation != Qt::Horizontal || role != Qt::DisplayRole) {
        return {};
    }
    switch (section) {
    case PathColumn:
        return tr("Path");
    case TypeColumn:
        return tr("Type");
    case SizeColumn:
        return tr("Size");
    case StoredColumn:
        return tr("Stored");
    case MachColumn:
        return tr("MACH");
    case SubsystemColumn:
        return tr("Subsystem");
    default:
        return {};
    }
}

QString EntryTableModel::typeText(const EntrySummarySnapshot &entry) const
{
    switch (entry.fileType) {
    case EntryFileType::Regular:
        return tr("File");
    case EntryFileType::Directory:
        return tr("Dir");
    case EntryFileType::SymbolicLink:
        return tr("Link");
    case EntryFileType::BlockDevice:
        return tr("Block");
    case EntryFileType::CharacterDevice:
        return tr("Char");
    case EntryFileType::Fifo:
        return tr("Fifo");
    case EntryFileType::Other:
        // The raw type letter is preserved, so it stays visible.
        return entry.fileTypeRaw;
    }
    Q_UNREACHABLE();
}

QString EntryTableModel::machToolTip(const EntrySummarySnapshot &entry) const
{
    QStringList lines = entry.mach;
    for (const QString &raw : entry.unresolvedMach) {
        lines.append(tr("Unresolved: %1").arg(raw));
    }
    return lines.join(QLatin1Char('\n'));
}
