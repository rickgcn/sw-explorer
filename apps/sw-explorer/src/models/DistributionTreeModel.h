#pragma once

#include "HierarchySnapshot.h"

#include <QAbstractItemModel>

#include <memory>
#include <vector>

// One node of the hierarchy tree. Items are owned by their parent via
// std::unique_ptr; the model's invisible root (id 0) owns the
// products.
struct HierarchyItem {
    quint64 id = 0;
    HierarchyKind kind = HierarchyKind::Product;

    QString name;
    QString title;
    quint64 entryCount = 0;

    bool descriptorPresent = false;
    bool idbPresent = false;

    HierarchyItem *parent = nullptr;
    std::vector<std::unique_ptr<HierarchyItem>> children;
};

// Presents the backend's product -> image -> subsystem hierarchy to a
// QTreeView. Fed with whole HierarchySnapshot values; a snapshot is
// validated completely before it replaces anything, so a malformed
// snapshot can never produce a partial render.
class DistributionTreeModel : public QAbstractItemModel
{
    Q_OBJECT

public:
    enum Column {
        NameColumn = 0,
        EntriesColumn = 1,
        ColumnCount = 2,
    };

    explicit DistributionTreeModel(QObject *parent = nullptr);

    // Replaces the current tree with the given snapshot.
    //
    // The snapshot is validated in full first (non-zero unique ids,
    // existing parents, product/image/subsystem nesting rules). If it
    // is invalid the current tree is kept untouched, false is
    // returned and, when errorMessage is not null, a description of
    // the problem is stored there.
    bool setHierarchy(const HierarchySnapshot &snapshot, QString *errorMessage = nullptr);

    // Backend object identity behind an index: the stable object id
    // (0 for invalid indexes), the tree level and the full dotted
    // identity (e.g. "eoe.sw.unix").
    quint64 objectId(const QModelIndex &index) const;
    HierarchyKind kind(const QModelIndex &index) const;
    QString identity(const QModelIndex &index) const;

    QModelIndex index(int row,
                      int column,
                      const QModelIndex &parent = QModelIndex()) const override;
    QModelIndex parent(const QModelIndex &child) const override;
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    int columnCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;
    QVariant headerData(int section,
                        Qt::Orientation orientation,
                        int role = Qt::DisplayRole) const override;

private:
    HierarchyItem *itemFromIndex(const QModelIndex &index) const;
    QString identityOf(const HierarchyItem *item) const;
    QString toolTipOf(const HierarchyItem *item) const;

    std::unique_ptr<HierarchyItem> m_root;
};
