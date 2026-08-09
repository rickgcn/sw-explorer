#include "HardwareProfileDialog.h"

#include "HardwareProfileDelegate.h"
#include "models/HardwareProfileModel.h"

#include <QDialogButtonBox>
#include <QHeaderView>
#include <QLabel>
#include <QMessageBox>
#include <QPushButton>
#include <QTableView>
#include <QVBoxLayout>

HardwareProfileDialog::HardwareProfileDialog(QWidget *parent)
    : QDialog(parent)
{
    setWindowTitle(tr("Hardware Profile"));
    resize(480, 320);

    auto *layout = new QVBoxLayout(this);

    auto *hint = new QLabel(
        tr("Hardware facts of the simulated target machine, one attribute value per row."),
        this);
    hint->setWordWrap(true);
    layout->addWidget(hint);

    m_model = new HardwareProfileModel(this);
    m_delegate = new HardwareProfileDelegate(this);
    m_view = new QTableView(this);
    m_view->setObjectName(QStringLiteral("profileTable"));
    m_view->setModel(m_model);
    m_view->setItemDelegate(m_delegate);
    m_view->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_view->setSelectionMode(QAbstractItemView::ExtendedSelection);
    m_view->horizontalHeader()->setSectionResizeMode(QHeaderView::Stretch);
    m_view->verticalHeader()->setVisible(false);
    layout->addWidget(m_view, 1);

    auto *rowButtons = new QHBoxLayout;
    m_addButton = new QPushButton(tr("+ Add"), this);
    m_addButton->setObjectName(QStringLiteral("addButton"));
    m_removeButton = new QPushButton(tr("Remove"), this);
    m_removeButton->setObjectName(QStringLiteral("removeButton"));
    m_clearButton = new QPushButton(tr("Clear"), this);
    m_clearButton->setObjectName(QStringLiteral("clearButton"));
    rowButtons->addWidget(m_addButton);
    rowButtons->addWidget(m_removeButton);
    rowButtons->addWidget(m_clearButton);
    rowButtons->addStretch();
    layout->addLayout(rowButtons);

    auto *buttonBox = new QDialogButtonBox(this);
    m_applyButton = buttonBox->addButton(QDialogButtonBox::Apply);
    m_applyButton->setObjectName(QStringLiteral("applyButton"));
    m_cancelButton = buttonBox->addButton(QDialogButtonBox::Cancel);
    m_cancelButton->setObjectName(QStringLiteral("cancelButton"));
    layout->addWidget(buttonBox);

    connect(m_addButton, &QPushButton::clicked, this, &HardwareProfileDialog::addRow);
    connect(m_removeButton, &QPushButton::clicked, this, &HardwareProfileDialog::removeSelectedRows);
    connect(m_clearButton, &QPushButton::clicked, this, &HardwareProfileDialog::clearRows);
    connect(m_applyButton, &QPushButton::clicked, this, &HardwareProfileDialog::apply);
    connect(m_cancelButton, &QPushButton::clicked, this, &QDialog::reject);
}

void HardwareProfileDialog::setProfile(const HardwareProfileSnapshot &profile)
{
    m_model->setProfile(profile);
}

void HardwareProfileDialog::setCandidates(const HardwareCandidatesSnapshot &candidates)
{
    m_delegate->setCandidates(candidates);
}

void HardwareProfileDialog::addRow()
{
    const QModelIndex added = m_model->addRow();
    m_view->setCurrentIndex(added);
    m_view->edit(added);
}

void HardwareProfileDialog::removeSelectedRows()
{
    QList<int> rows;
    const QModelIndexList selected = m_view->selectionModel()->selectedRows();
    rows.reserve(selected.size());
    for (const QModelIndex &index : selected) {
        rows.append(index.row());
    }
    m_model->removeRows(rows);
}

void HardwareProfileDialog::clearRows()
{
    m_model->clear();
}

void HardwareProfileDialog::apply()
{
    // A still-open cell editor may not have committed yet (the user
    // went straight from typing to Apply, and FocusOut commits are
    // deferred); pull its text in explicitly before validating.
    const QModelIndex current = m_view->currentIndex();
    if (QWidget *editor = m_view->indexWidget(current)) {
        m_delegate->setModelData(editor, m_model, current);
    }

    // Rows where both fields are blank are dropped. An empty value is
    // a genuine hardware fact (the media carry restrictions like
    // `GFXBOARD=`), but a value without an attribute is a mistake
    // worth pointing out, and never reaches the backend.
    HardwareProfileSnapshot profile;
    for (int row = 0; row < m_model->rowCount(); ++row) {
        const QString attribute =
            m_model->index(row, HardwareProfileModel::AttributeColumn).data().toString().trimmed();
        const QString value =
            m_model->index(row, HardwareProfileModel::ValueColumn).data().toString().trimmed();
        if (attribute.isEmpty() && value.isEmpty()) {
            continue;
        }
        if (attribute.isEmpty()) {
            QMessageBox::warning(
                this,
                tr("Hardware Profile"),
                tr("Row %1 has a value but no attribute.").arg(row + 1));
            return;
        }
        profile.append({attribute, value});
    }
    emit profileApplied(profile);
    accept();
}
