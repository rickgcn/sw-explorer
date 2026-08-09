#pragma once

#include "HardwareSnapshot.h"

#include <QStyledItemDelegate>

class QComboBox;

// Editable-combobox delegate for the hardware profile table.
//
// Both columns offer suggestions but never require them: the
// comboboxes are editable, so values the current distribution does
// not know about can always be typed in. The Attribute column lists
// the canonical MACH attribute names plus any unknown attribute the
// committed distribution actually carries; the Value column lists
// the values discovered for the row's current attribute. The cell's
// current text is always present in its own suggestion list, even
// when it is not part of the candidates.
//
// The empty value is a genuine hardware fact (the media carry
// restrictions like `GFXBOARD=`); suggestions show it as an "(empty)"
// placeholder so it does not look like an untouched cell, while the
// stored value stays the empty string.
class HardwareProfileDelegate : public QStyledItemDelegate
{
    Q_OBJECT

public:
    explicit HardwareProfileDelegate(QObject *parent = nullptr);

    // The distribution-discovered suggestions; canonical attribute
    // names are always offered regardless.
    void setCandidates(const HardwareCandidatesSnapshot &candidates);

    QWidget *createEditor(QWidget *parent,
                          const QStyleOptionViewItem &option,
                          const QModelIndex &index) const override;
    void setEditorData(QWidget *editor, const QModelIndex &index) const override;
    void setModelData(QWidget *editor,
                      QAbstractItemModel *model,
                      const QModelIndex &index) const override;

private:
    void addSuggestion(QComboBox *combo, const QString &value) const;

    HardwareCandidatesSnapshot m_candidates;
};
