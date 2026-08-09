#pragma once

#include "EntrySnapshot.h"
#include "HardwareSnapshot.h"

#include <QAbstractTableModel>
#include <QHash>
#include <QSet>

// The four hardware-selection states of one entry row, defined
// purely by set membership in the current SelectionSnapshot:
//
//   selected     conflict   status
//   yes          no         Selected
//   yes          yes        SelectedConflict
//   no           yes        Unresolved
//   no           no         NotSelected
//
// A conflict candidate is not automatically "not selected": the core
// keeps conflict candidates in the selected set, so a three-state
// model would lose semantics.
enum class EntrySelectionStatus {
    Selected,
    SelectedConflict,
    Unresolved,
    NotSelected,
};

// Presents the entries of one hierarchy scope (product, image or
// subsystem) to a QTableView, in exact IDB order. Fed with whole
// EntryListSnapshot values; a snapshot is validated completely
// before it replaces anything, so a malformed snapshot can never
// produce a partial render.
//
// Rows are IDB records: duplicate paths are legal and never
// rejected. Sorting stays off everywhere — the view order is the
// IDB order.
//
// The hardware selection is a pure overlay: setSelectionOverlay()
// only changes what the Status column reports for the existing rows,
// never the rows themselves. The overlay survives setEntries(), so a
// reloaded scope or a fresh search result picks the current
// selection up automatically.
class EntryTableModel : public QAbstractTableModel
{
    Q_OBJECT

public:
    enum Column {
        StatusColumn = 0,
        PathColumn = 1,
        TypeColumn = 2,
        SizeColumn = 3,
        StoredColumn = 4,
        MachColumn = 5,
        SubsystemColumn = 6,
        ColumnCount = 7,
    };

    // Custom roles carrying the raw values behind the display text.
    enum Role {
        SizeBytesRole = Qt::UserRole,
        StoredBytesRole,
        // The EntrySelectionStatus of the row, as int.
        SelectionStatusRole,
    };

    explicit EntryTableModel(QObject *parent = nullptr);

    // Replaces the rows with the given snapshot.
    //
    // The snapshot is validated in full first: (productId, entryId)
    // pairs must be unique within the snapshot. If it is invalid the
    // current rows are kept untouched, false is returned and, when
    // errorMessage is not null, a description of the problem is
    // stored there.
    //
    // The selection overlay is kept and applies to the new rows.
    bool setEntries(const EntryListSnapshot &snapshot, QString *errorMessage = nullptr);

    // Overlays a hardware selection on the current rows. Only the
    // Status column changes: no rows are added, removed, reordered or
    // reset.
    void setSelectionOverlay(const SelectionSnapshot &selection);
    // Removes the selection overlay; the Status column reports
    // nothing until the next setSelectionOverlay().
    void clearSelectionOverlay();
    // Whether a selection overlay is currently applied.
    bool hasSelectionOverlay() const;

    // The selection status of one entry key under the current
    // overlay; NotSelected when no overlay is applied.
    EntrySelectionStatus statusForKey(const EntryKey &key) const;
    // Number of current rows whose key is in the selected set.
    int selectedRowCount() const;
    // Number of current rows whose key is a candidate of any
    // conflict.
    int conflictedRowCount() const;

    // Backend identity behind an index, taken from the row's own
    // data — never derived from the row number.
    EntryKey entryKey(const QModelIndex &index) const;

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    int columnCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QVariant headerData(int section,
                        Qt::Orientation orientation,
                        int role = Qt::DisplayRole) const override;

private:
    QString statusText(EntrySelectionStatus status) const;
    QString statusToolTip(const EntrySummarySnapshot &entry) const;
    QString typeText(const EntrySummarySnapshot &entry) const;
    QString machToolTip(const EntrySummarySnapshot &entry) const;
    // Emits dataChanged for the whole Status column.
    void emitStatusChanged();

    EntryListSnapshot m_entries;

    bool m_overlayActive = false;
    QSet<EntryKey> m_selected;
    // Entry key -> the paths it conflicts over (for the tooltip).
    QHash<EntryKey, QStringList> m_conflictPaths;
};
