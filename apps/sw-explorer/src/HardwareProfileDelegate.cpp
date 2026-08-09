#include "HardwareProfileDelegate.h"

#include "models/HardwareProfileModel.h"

#include <QComboBox>

namespace {

// The attribute names the MACH grammar defines. This is grammar
// knowledge, not a machine database: it is offered even with no
// distribution loaded.
const QStringList &canonicalAttributes()
{
    static const QStringList attributes = {QStringLiteral("CPUBOARD"),
                                           QStringLiteral("CPUARCH"),
                                           QStringLiteral("GFXBOARD"),
                                           QStringLiteral("SUBGR"),
                                           QStringLiteral("VIDEO"),
                                           QStringLiteral("MODE"),
                                           QStringLiteral("TARGOS"),
                                           QStringLiteral("DISTOS")};
    return attributes;
}

} // namespace

HardwareProfileDelegate::HardwareProfileDelegate(QObject *parent)
    : QStyledItemDelegate(parent)
{
}

void HardwareProfileDelegate::setCandidates(const HardwareCandidatesSnapshot &candidates)
{
    m_candidates = candidates;
}

// Adds one suggestion carrying its real value as user data; the empty
// value gets a visible "(empty)" placeholder so it does not look like
// an untouched cell. Existing real values are not duplicated.
void HardwareProfileDelegate::addSuggestion(QComboBox *combo, const QString &value) const
{
    for (int i = 0; i < combo->count(); ++i) {
        if (combo->itemData(i).toString() == value) {
            return;
        }
    }
    combo->addItem(value.isEmpty() ? tr("(empty)") : value, value);
}

QWidget *HardwareProfileDelegate::createEditor(QWidget *parent,
                                               const QStyleOptionViewItem &option,
                                               const QModelIndex &index) const
{
    Q_UNUSED(option);
    auto *combo = new QComboBox(parent);
    // Suggestions are a convenience, never a whitelist.
    combo->setEditable(true);

    const QString own = index.data().toString();
    if (index.column() == HardwareProfileModel::AttributeColumn) {
        for (const QString &attribute : canonicalAttributes()) {
            addSuggestion(combo, attribute);
        }
        // Unknown attributes the committed distribution carries join
        // the canonical names.
        for (const HardwareCandidateSetSnapshot &set : m_candidates) {
            if (!set.attribute.isEmpty()) {
                addSuggestion(combo, set.attribute);
            }
        }
        // The cell's own text is always a valid suggestion.
        if (!own.isEmpty()) {
            addSuggestion(combo, own);
        }
    } else {
        // Value suggestions follow the row's current attribute.
        const QString attribute =
            index.siblingAtColumn(HardwareProfileModel::AttributeColumn).data().toString();
        for (const HardwareCandidateSetSnapshot &set : m_candidates) {
            if (set.attribute == attribute) {
                for (const QString &value : set.values) {
                    addSuggestion(combo, value);
                }
                break;
            }
        }
        // The cell's own text is always a valid suggestion — the
        // empty value included.
        addSuggestion(combo, own);
    }
    return combo;
}

void HardwareProfileDelegate::setEditorData(QWidget *editor, const QModelIndex &index) const
{
    auto *combo = qobject_cast<QComboBox *>(editor);
    if (combo == nullptr) {
        return;
    }
    // Select the suggestion carrying the cell's real value, so the
    // empty value shows its "(empty)" placeholder instead of a blank
    // line edit.
    const QString text = index.data().toString();
    for (int i = 0; i < combo->count(); ++i) {
        if (combo->itemData(i).toString() == text) {
            combo->setCurrentIndex(i);
            return;
        }
    }
    combo->setCurrentText(text);
}

void HardwareProfileDelegate::setModelData(QWidget *editor,
                                           QAbstractItemModel *model,
                                           const QModelIndex &index) const
{
    auto *combo = qobject_cast<QComboBox *>(editor);
    if (combo == nullptr) {
        return;
    }
    // A suggestion stores its real value (the "(empty)" placeholder
    // stores an empty string); hand-typed text is used verbatim.
    const int current = combo->currentIndex();
    if (current >= 0 && combo->itemText(current) == combo->currentText()) {
        model->setData(index, combo->itemData(current), Qt::EditRole);
        return;
    }
    model->setData(index, combo->currentText(), Qt::EditRole);
}
