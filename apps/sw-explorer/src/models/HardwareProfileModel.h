#pragma once

#include "HardwareSnapshot.h"

#include <QAbstractTableModel>

// The editable two-column table behind the hardware profile dialog:
// one row per attribute/value pair of the simulated target machine.
//
// The same attribute may appear on several rows with different
// values (IRIX can report e.g. both CPUARCH=MIPS2 and CPUARCH=R4000
// for one machine); exact duplicate pairs are kept as-is and
// deduplicated harmlessly by the core profile builder. The model
// stores plain text only — it knows nothing about MACH expressions.
class HardwareProfileModel : public QAbstractTableModel
{
    Q_OBJECT

public:
    enum Column {
        AttributeColumn = 0,
        ValueColumn = 1,
        ColumnCount = 2,
    };

    explicit HardwareProfileModel(QObject *parent = nullptr);

    // Replaces all rows with the given profile.
    void setProfile(const HardwareProfileSnapshot &profile);
    // The current rows as a profile, verbatim.
    HardwareProfileSnapshot profile() const;

    // Appends one empty row and returns its index.
    QModelIndex addRow();
    // Removes the given rows; invalid row numbers are ignored.
    void removeRows(const QList<int> &rows);
    // Removes all rows.
    void clear();

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    int columnCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QVariant headerData(int section,
                        Qt::Orientation orientation,
                        int role = Qt::DisplayRole) const override;
    Qt::ItemFlags flags(const QModelIndex &index) const override;
    bool setData(const QModelIndex &index, const QVariant &value, int role = Qt::EditRole) override;

private:
    HardwareProfileSnapshot m_rows;
};
