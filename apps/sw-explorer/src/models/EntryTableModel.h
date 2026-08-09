#pragma once

#include "EntrySnapshot.h"

#include <QAbstractTableModel>

// Backend identity of one entry row: the product's hierarchy object
// id plus the sw-core entry id inside that product. EntryId 0 is a
// perfectly valid first entry; it is never an error marker.
struct EntryKey {
    quint64 productId = 0;
    quint64 entryId = 0;
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
class EntryTableModel : public QAbstractTableModel
{
    Q_OBJECT

public:
    enum Column {
        PathColumn = 0,
        TypeColumn = 1,
        SizeColumn = 2,
        StoredColumn = 3,
        MachColumn = 4,
        SubsystemColumn = 5,
        ColumnCount = 6,
    };

    // Custom roles carrying the raw values behind the display text.
    enum Role {
        SizeBytesRole = Qt::UserRole,
        StoredBytesRole,
    };

    explicit EntryTableModel(QObject *parent = nullptr);

    // Replaces the rows with the given snapshot.
    //
    // The snapshot is validated in full first: (productId, entryId)
    // pairs must be unique within the snapshot. If it is invalid the
    // current rows are kept untouched, false is returned and, when
    // errorMessage is not null, a description of the problem is
    // stored there.
    bool setEntries(const EntryListSnapshot &snapshot, QString *errorMessage = nullptr);

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
    QString typeText(const EntrySummarySnapshot &entry) const;
    QString machToolTip(const EntrySummarySnapshot &entry) const;

    EntryListSnapshot m_entries;
};
