#include "EntryBrowserWidget.h"

#include "models/EntryTableModel.h"

#include <QHeaderView>
#include <QItemSelectionModel>
#include <QLabel>
#include <QStackedWidget>
#include <QTableView>
#include <QVBoxLayout>

namespace {

QWidget *centeredPage(const QString &objectName, const QString &text)
{
    auto *page = new QWidget;
    page->setObjectName(objectName);
    auto *layout = new QVBoxLayout(page);
    auto *label = new QLabel(text);
    label->setAlignment(Qt::AlignCenter);
    layout->addWidget(label);
    return page;
}

} // namespace

EntryBrowserWidget::EntryBrowserWidget(QWidget *parent)
    : QWidget(parent)
{
    m_stack = new QStackedWidget(this);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->addWidget(m_stack);

    m_emptyPage = centeredPage(QStringLiteral("emptyPage"), tr("Select an item"));
    m_loadingPage = centeredPage(QStringLiteral("loadingPage"), tr("Loading..."));

    m_errorPage = new QWidget;
    m_errorPage->setObjectName(QStringLiteral("errorPage"));
    auto *errorLayout = new QVBoxLayout(m_errorPage);
    errorLayout->addStretch();
    auto *errorTitle = new QLabel(tr("Unable to load entries"), m_errorPage);
    QFont titleFont = errorTitle->font();
    titleFont.setBold(true);
    errorTitle->setFont(titleFont);
    errorTitle->setAlignment(Qt::AlignCenter);
    m_errorMessage = new QLabel(m_errorPage);
    m_errorMessage->setObjectName(QStringLiteral("errorMessage"));
    m_errorMessage->setTextFormat(Qt::PlainText);
    m_errorMessage->setAlignment(Qt::AlignCenter);
    m_errorMessage->setWordWrap(true);
    errorLayout->addWidget(errorTitle);
    errorLayout->addWidget(m_errorMessage);
    errorLayout->addStretch();

    // The table page: a small header naming the scope, the entry
    // table itself, and a footer with the entry count.
    m_tablePage = new QWidget;
    m_tablePage->setObjectName(QStringLiteral("tablePage"));
    auto *tableLayout = new QVBoxLayout(m_tablePage);
    tableLayout->setContentsMargins(4, 4, 4, 4);

    auto *headerRow = new QWidget(m_tablePage);
    auto *headerLayout = new QVBoxLayout(headerRow);
    headerLayout->setContentsMargins(4, 2, 4, 2);
    auto *titleLabel = new QLabel(tr("Files"), headerRow);
    QFont headerFont = titleLabel->font();
    headerFont.setBold(true);
    titleLabel->setFont(headerFont);
    m_scopeLabel = new QLabel(headerRow);
    m_scopeLabel->setObjectName(QStringLiteral("scopeLabel"));
    m_scopeLabel->setTextFormat(Qt::PlainText);
    m_scopeLabel->setTextInteractionFlags(Qt::TextSelectableByMouse);
    headerLayout->addWidget(titleLabel);
    headerLayout->addWidget(m_scopeLabel);
    tableLayout->addWidget(headerRow);

    m_model = new EntryTableModel(this);
    m_tableView = new QTableView(m_tablePage);
    m_tableView->setObjectName(QStringLiteral("entryTable"));
    m_tableView->setModel(m_model);
    // The view order is the IDB order; sorting stays off.
    m_tableView->setSortingEnabled(false);
    m_tableView->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_tableView->setSelectionMode(QAbstractItemView::SingleSelection);
    m_tableView->setShowGrid(false);
    m_tableView->setAlternatingRowColors(true);
    m_tableView->setTextElideMode(Qt::ElideMiddle);
    m_tableView->verticalHeader()->setVisible(false);
    m_tableView->horizontalHeader()->setSectionResizeMode(EntryTableModel::PathColumn,
                                                          QHeaderView::Stretch);
    for (int column = EntryTableModel::TypeColumn; column < EntryTableModel::ColumnCount;
         ++column) {
        m_tableView->horizontalHeader()->setSectionResizeMode(column,
                                                              QHeaderView::ResizeToContents);
    }
    tableLayout->addWidget(m_tableView, 1);

    m_countLabel = new QLabel(m_tablePage);
    m_countLabel->setObjectName(QStringLiteral("countLabel"));
    m_countLabel->setTextFormat(Qt::PlainText);
    tableLayout->addWidget(m_countLabel);

    m_stack->addWidget(m_emptyPage);
    m_stack->addWidget(m_loadingPage);
    m_stack->addWidget(m_tablePage);
    m_stack->addWidget(m_errorPage);
    m_stack->setCurrentWidget(m_emptyPage);

    connect(m_tableView->selectionModel(),
            &QItemSelectionModel::currentRowChanged,
            this,
            &EntryBrowserWidget::onCurrentRowChanged);
}

void EntryBrowserWidget::showEmpty()
{
    m_scopeLabel->clear();
    m_countLabel->clear();
    m_stack->setCurrentWidget(m_emptyPage);
}

void EntryBrowserWidget::showLoading(const QString &scopeName)
{
    m_scopeLabel->setText(scopeName);
    m_stack->setCurrentWidget(m_loadingPage);
}

void EntryBrowserWidget::showEntries(const EntryListSnapshot &entries)
{
    QString error;
    if (!m_model->setEntries(entries, &error)) {
        // An invalid snapshot replaces nothing; surface the problem
        // on the error page instead of rendering partial data.
        showError(tr("The backend returned an invalid entry list: %1").arg(error));
        return;
    }
    m_countLabel->setText(tr("%n entries", nullptr, static_cast<int>(entries.size())));
    m_stack->setCurrentWidget(m_tablePage);
}

void EntryBrowserWidget::showError(const QString &message)
{
    m_errorMessage->setText(message);
    m_stack->setCurrentWidget(m_errorPage);
}

void EntryBrowserWidget::onCurrentRowChanged(const QModelIndex &current,
                                             const QModelIndex &previous)
{
    Q_UNUSED(previous);
    // Only real row picks emit; a model reset that cleared the
    // selection arrives here with an invalid index and stays silent.
    if (!current.isValid()) {
        return;
    }
    const EntryKey key = m_model->entryKey(current);
    emit entrySelected(key.productId, key.entryId);
}
