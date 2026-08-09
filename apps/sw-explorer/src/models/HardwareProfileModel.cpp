#include "HardwareProfileModel.h"

#include <algorithm>
#include <functional>

HardwareProfileModel::HardwareProfileModel(QObject *parent)
    : QAbstractTableModel(parent)
{
}

void HardwareProfileModel::setProfile(const HardwareProfileSnapshot &profile)
{
    beginResetModel();
    m_rows = profile;
    endResetModel();
}

HardwareProfileSnapshot HardwareProfileModel::profile() const
{
    return m_rows;
}

QModelIndex HardwareProfileModel::addRow()
{
    const int row = static_cast<int>(m_rows.size());
    beginInsertRows({}, row, row);
    m_rows.append(HardwareValueSnapshot{});
    endInsertRows();
    return index(row, AttributeColumn);
}

void HardwareProfileModel::removeRows(const QList<int> &rows)
{
    // Highest row first, so earlier removals never renumber later
    // ones.
    QList<int> sorted = rows;
    std::sort(sorted.begin(), sorted.end(), std::greater<int>());
    for (const int row : sorted) {
        if (row < 0 || row >= m_rows.size()) {
            continue;
        }
        beginRemoveRows({}, row, row);
        m_rows.removeAt(row);
        endRemoveRows();
    }
}

void HardwareProfileModel::clear()
{
    setProfile({});
}

int HardwareProfileModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : static_cast<int>(m_rows.size());
}

int HardwareProfileModel::columnCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : ColumnCount;
}

QVariant HardwareProfileModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.model() != this || index.row() >= m_rows.size()) {
        return {};
    }
    if (role != Qt::DisplayRole && role != Qt::EditRole) {
        return {};
    }
    const HardwareValueSnapshot &row = m_rows.at(index.row());
    switch (index.column()) {
    case AttributeColumn:
        return row.attribute;
    case ValueColumn:
        return row.value;
    default:
        return {};
    }
}

QVariant HardwareProfileModel::headerData(int section,
                                          Qt::Orientation orientation,
                                          int role) const
{
    if (orientation != Qt::Horizontal || role != Qt::DisplayRole) {
        return {};
    }
    switch (section) {
    case AttributeColumn:
        return tr("Attribute");
    case ValueColumn:
        return tr("Value");
    default:
        return {};
    }
}

Qt::ItemFlags HardwareProfileModel::flags(const QModelIndex &index) const
{
    if (!index.isValid()) {
        return Qt::NoItemFlags;
    }
    return Qt::ItemIsSelectable | Qt::ItemIsEnabled | Qt::ItemIsEditable;
}

bool HardwareProfileModel::setData(const QModelIndex &index, const QVariant &value, int role)
{
    if (!index.isValid() || index.model() != this || index.row() >= m_rows.size()
        || role != Qt::EditRole) {
        return false;
    }
    // Outer whitespace is never meaningful in a hardware fact.
    const QString text = value.toString().trimmed();
    HardwareValueSnapshot &row = m_rows[index.row()];
    switch (index.column()) {
    case AttributeColumn:
        row.attribute = text;
        break;
    case ValueColumn:
        row.value = text;
        break;
    default:
        return false;
    }
    emit dataChanged(index, index, {Qt::DisplayRole, Qt::EditRole});
    return true;
}
