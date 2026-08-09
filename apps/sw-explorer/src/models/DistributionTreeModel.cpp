#include "DistributionTreeModel.h"

#include <QHash>
#include <QStringList>

DistributionTreeModel::DistributionTreeModel(QObject *parent)
    : QAbstractItemModel(parent)
    , m_root(std::make_unique<HierarchyItem>())
{
}

bool DistributionTreeModel::setHierarchy(const HierarchySnapshot &snapshot, QString *errorMessage)
{
    const auto fail = [errorMessage](const QString &message) {
        if (errorMessage != nullptr) {
            *errorMessage = message;
        }
        return false;
    };

    // Build into a fresh tree: only a fully valid snapshot ever
    // touches the live model.
    auto root = std::make_unique<HierarchyItem>();

    // Pass 1: create every item, checking that ids are non-zero and
    // unique.
    std::vector<std::unique_ptr<HierarchyItem>> items;
    items.reserve(static_cast<std::size_t>(snapshot.size()));
    QHash<quint64, HierarchyItem *> byId;
    byId.reserve(snapshot.size());
    for (const HierarchyNodeSnapshot &node : snapshot) {
        if (node.id == 0) {
            return fail(tr("object id 0 is reserved for the invisible root"));
        }
        if (byId.contains(node.id)) {
            return fail(tr("duplicate object id %1").arg(node.id));
        }

        auto item = std::make_unique<HierarchyItem>();
        item->id = node.id;
        item->kind = node.kind;
        item->name = node.name;
        item->title = node.title;
        item->entryCount = node.entryCount;
        item->descriptorPresent = node.descriptorPresent;
        item->idbPresent = node.idbPresent;
        byId.insert(node.id, item.get());
        items.push_back(std::move(item));
    }

    // Pass 2: link every item to its parent, checking that the parent
    // exists and has the kind the hierarchy demands.
    for (qsizetype position = 0; position < snapshot.size(); ++position) {
        const HierarchyNodeSnapshot &node = snapshot[position];
        HierarchyItem *parentItem = nullptr;
        switch (node.kind) {
        case HierarchyKind::Product:
            if (node.parentId != 0) {
                return fail(tr("product %1 (%2) must not have a parent")
                                .arg(node.id)
                                .arg(node.name));
            }
            parentItem = root.get();
            break;
        case HierarchyKind::Image:
        case HierarchyKind::Subsystem: {
            if (node.parentId == 0) {
                return fail(tr("%1 %2 (%3) has no parent")
                                .arg(node.kind == HierarchyKind::Image ? tr("image")
                                                                       : tr("subsystem"))
                                .arg(node.id)
                                .arg(node.name));
            }
            HierarchyItem *candidate = byId.value(node.parentId, nullptr);
            if (candidate == nullptr) {
                return fail(tr("node %1 (%2) references missing parent %3")
                                .arg(node.id)
                                .arg(node.name)
                                .arg(node.parentId));
            }
            const HierarchyKind required =
                node.kind == HierarchyKind::Image ? HierarchyKind::Product
                                                  : HierarchyKind::Image;
            if (candidate->kind != required) {
                return fail(tr("node %1 (%2) hangs below the wrong kind of parent")
                                .arg(node.id)
                                .arg(node.name));
            }
            parentItem = candidate;
            break;
        }
        }

        items[static_cast<std::size_t>(position)]->parent = parentItem;
        parentItem->children.push_back(std::move(items[static_cast<std::size_t>(position)]));
    }

    beginResetModel();
    m_root = std::move(root);
    endResetModel();
    return true;
}

quint64 DistributionTreeModel::objectId(const QModelIndex &index) const
{
    const HierarchyItem *item = itemFromIndex(index);
    return item != nullptr ? item->id : 0;
}

HierarchyKind DistributionTreeModel::kind(const QModelIndex &index) const
{
    const HierarchyItem *item = itemFromIndex(index);
    return item != nullptr ? item->kind : HierarchyKind::Product;
}

QString DistributionTreeModel::identity(const QModelIndex &index) const
{
    const HierarchyItem *item = itemFromIndex(index);
    return item != nullptr ? identityOf(item) : QString();
}

QModelIndex DistributionTreeModel::index(int row, int column, const QModelIndex &parent) const
{
    if (row < 0 || column < 0 || column >= ColumnCount) {
        return {};
    }
    HierarchyItem *parentItem = parent.isValid() ? itemFromIndex(parent) : m_root.get();
    if (parentItem == nullptr
        || static_cast<std::size_t>(row) >= parentItem->children.size()) {
        return {};
    }
    return createIndex(row, column, parentItem->children[static_cast<std::size_t>(row)].get());
}

QModelIndex DistributionTreeModel::parent(const QModelIndex &child) const
{
    HierarchyItem *item = itemFromIndex(child);
    if (item == nullptr) {
        return {};
    }
    HierarchyItem *parentItem = item->parent;
    if (parentItem == nullptr || parentItem == m_root.get() || parentItem->parent == nullptr) {
        return {};
    }
    const auto &siblings = parentItem->parent->children;
    for (std::size_t row = 0; row < siblings.size(); ++row) {
        if (siblings[row].get() == parentItem) {
            return createIndex(static_cast<int>(row), 0, parentItem);
        }
    }
    return {};
}

int DistributionTreeModel::rowCount(const QModelIndex &parent) const
{
    if (parent.column() > 0) {
        return 0;
    }
    HierarchyItem *parentItem = parent.isValid() ? itemFromIndex(parent) : m_root.get();
    return parentItem != nullptr ? static_cast<int>(parentItem->children.size()) : 0;
}

int DistributionTreeModel::columnCount(const QModelIndex &parent) const
{
    Q_UNUSED(parent);
    return ColumnCount;
}

QVariant DistributionTreeModel::data(const QModelIndex &index, int role) const
{
    const HierarchyItem *item = itemFromIndex(index);
    if (item == nullptr) {
        return {};
    }

    switch (role) {
    case Qt::DisplayRole:
        if (index.column() == NameColumn) {
            return item->name;
        }
        return QVariant::fromValue(item->entryCount);
    case Qt::ToolTipRole:
        return toolTipOf(item);
    case Qt::TextAlignmentRole:
        if (index.column() == EntriesColumn) {
            return QVariant::fromValue(Qt::AlignRight | Qt::AlignVCenter);
        }
        return {};
    default:
        return {};
    }
}

QVariant DistributionTreeModel::headerData(int section,
                                           Qt::Orientation orientation,
                                           int role) const
{
    if (orientation != Qt::Horizontal || role != Qt::DisplayRole) {
        return {};
    }
    switch (section) {
    case NameColumn:
        return tr("Name");
    case EntriesColumn:
        return tr("Entries");
    default:
        return {};
    }
}

HierarchyItem *DistributionTreeModel::itemFromIndex(const QModelIndex &index) const
{
    if (!index.isValid() || index.model() != this) {
        return nullptr;
    }
    return static_cast<HierarchyItem *>(index.internalPointer());
}

QString DistributionTreeModel::identityOf(const HierarchyItem *item) const
{
    // Image names are already fully qualified (`eoe.sw`); products and
    // images identify themselves. Subsystems extend their image.
    if (item->parent == nullptr || item->parent == m_root.get()
        || item->kind == HierarchyKind::Image) {
        return item->name;
    }
    return identityOf(item->parent) + QLatin1Char('.') + item->name;
}

QString DistributionTreeModel::toolTipOf(const HierarchyItem *item) const
{
    QStringList lines;
    lines.append(identityOf(item));
    if (!item->title.isEmpty()) {
        lines.append(item->title);
    }
    lines.append(tr("Descriptor: %1").arg(item->descriptorPresent ? tr("yes") : tr("no")));
    lines.append(tr("IDB: %1").arg(item->idbPresent ? tr("yes") : tr("no")));
    lines.append(tr("Entries: %1").arg(item->entryCount));
    return lines.join(QLatin1Char('\n'));
}
