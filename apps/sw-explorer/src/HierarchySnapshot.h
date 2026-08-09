#pragma once

#include <QList>
#include <QMetaType>
#include <QString>

// Qt-native mirror of the sw::HierarchyNode bridge struct.
//
// This is the only shape in which the GUI thread ever sees the
// distribution hierarchy: plain Qt value types. rust:: types exist
// solely inside BackendWorker, which performs the conversion.
enum class HierarchyKind {
    Product,
    Image,
    Subsystem,
};

struct HierarchyNodeSnapshot {
    // Stable, non-zero object id assigned by the backend. Zero is
    // reserved for the model's invisible root.
    quint64 id = 0;
    // Id of the parent node, or zero for products.
    quint64 parentId = 0;
    HierarchyKind kind = HierarchyKind::Product;

    // Display name: product name, image file name or short subsystem
    // name.
    QString name;
    // Human-readable title, empty when the object has none.
    QString title;

    quint64 entryCount = 0;

    bool descriptorPresent = false;
    bool idbPresent = false;
};

using HierarchySnapshot = QList<HierarchyNodeSnapshot>;

Q_DECLARE_METATYPE(HierarchyNodeSnapshot)
Q_DECLARE_METATYPE(HierarchySnapshot)
