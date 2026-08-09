#include "EntryTableModel.h"

#include "ByteFormat.h"

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
    QSet<EntryKey> seenKeys;
    seenKeys.reserve(snapshot.size());
    for (const EntrySummarySnapshot &entry : snapshot) {
        const EntryKey key{entry.productId, entry.entryId};
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

void EntryTableModel::setSelectionOverlay(const SelectionSnapshot &selection)
{
    // Membership sets only: the rows themselves are never touched,
    // so the overlay cannot reorder, filter or duplicate anything.
    QSet<EntryKey> selected;
    selected.reserve(selection.selected.size());
    for (const EntryKey &key : selection.selected) {
        selected.insert(key);
    }
    QHash<EntryKey, QStringList> conflictPaths;
    for (const SelectionConflictSnapshot &conflict : selection.conflicts) {
        for (const EntryKey &key : conflict.candidates) {
            conflictPaths[key].append(conflict.path);
        }
    }

    m_selected = std::move(selected);
    m_conflictPaths = std::move(conflictPaths);
    m_overlayActive = true;
    emitStatusChanged();
}

void EntryTableModel::clearSelectionOverlay()
{
    m_selected.clear();
    m_conflictPaths.clear();
    m_overlayActive = false;
    emitStatusChanged();
}

bool EntryTableModel::hasSelectionOverlay() const
{
    return m_overlayActive;
}

EntrySelectionStatus EntryTableModel::statusForKey(const EntryKey &key) const
{
    if (!m_overlayActive) {
        return EntrySelectionStatus::NotSelected;
    }
    const bool selected = m_selected.contains(key);
    const bool conflicted = m_conflictPaths.contains(key);
    if (selected && conflicted) {
        return EntrySelectionStatus::SelectedConflict;
    }
    if (selected) {
        return EntrySelectionStatus::Selected;
    }
    if (conflicted) {
        return EntrySelectionStatus::Unresolved;
    }
    return EntrySelectionStatus::NotSelected;
}

int EntryTableModel::selectedRowCount() const
{
    if (!m_overlayActive) {
        return 0;
    }
    int count = 0;
    for (const EntrySummarySnapshot &entry : m_entries) {
        if (m_selected.contains({entry.productId, entry.entryId})) {
            ++count;
        }
    }
    return count;
}

int EntryTableModel::conflictedRowCount() const
{
    if (!m_overlayActive) {
        return 0;
    }
    int count = 0;
    for (const EntrySummarySnapshot &entry : m_entries) {
        if (m_conflictPaths.contains({entry.productId, entry.entryId})) {
            ++count;
        }
    }
    return count;
}

void EntryTableModel::emitStatusChanged()
{
    if (m_entries.isEmpty()) {
        return;
    }
    emit dataChanged(index(0, StatusColumn),
                     index(static_cast<int>(m_entries.size()) - 1, StatusColumn),
                     {Qt::DisplayRole, Qt::ToolTipRole, SelectionStatusRole});
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
        case StatusColumn:
            if (m_overlayActive) {
                return statusText(statusForKey({entry.productId, entry.entryId}));
            }
            return {};
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
        case StatusColumn:
            if (m_overlayActive) {
                return statusToolTip(entry);
            }
            return {};
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
    case SelectionStatusRole:
        return QVariant::fromValue(
            static_cast<int>(statusForKey({entry.productId, entry.entryId})));
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
    case StatusColumn:
        return tr("Status");
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

QString EntryTableModel::statusText(EntrySelectionStatus status) const
{
    switch (status) {
    case EntrySelectionStatus::Selected:
        return tr("Selected");
    case EntrySelectionStatus::SelectedConflict:
        return tr("Selected / conflict");
    case EntrySelectionStatus::Unresolved:
        return tr("Unresolved");
    case EntrySelectionStatus::NotSelected:
        return tr("Not selected");
    }
    Q_UNREACHABLE();
}

QString EntryTableModel::statusToolTip(const EntrySummarySnapshot &entry) const
{
    const EntryKey key{entry.productId, entry.entryId};
    switch (statusForKey(key)) {
    case EntrySelectionStatus::Selected:
        return tr("This IDB record is selected for the current hardware profile.");
    case EntrySelectionStatus::SelectedConflict: {
        const QStringList paths = m_conflictPaths.value(key);
        return tr("This IDB record is selected for the current hardware profile, but "
                  "participates in a selection conflict for %1.")
            .arg(paths.join(QStringLiteral(", ")));
    }
    case EntrySelectionStatus::Unresolved: {
        const QStringList paths = m_conflictPaths.value(key);
        return tr("The hardware applicability of this record cannot be determined: it "
                  "participates in a selection conflict for %1 and is not selected.")
            .arg(paths.join(QStringLiteral(", ")));
    }
    case EntrySelectionStatus::NotSelected:
        return tr("This IDB record does not apply to the current hardware profile.");
    }
    Q_UNREACHABLE();
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
