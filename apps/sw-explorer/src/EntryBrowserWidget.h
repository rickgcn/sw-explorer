#pragma once

#include "EntrySnapshot.h"

#include <QWidget>

QT_BEGIN_NAMESPACE
class QLabel;
class QModelIndex;
class QStackedWidget;
class QTableView;
QT_END_NAMESPACE

class EntryTableModel;

// The middle pane: the IDB entries of the currently selected
// product / image / subsystem, shown as one table in exact IDB
// order.
//
// The widget is a QStackedWidget with Empty / Loading / Table /
// Error pages. Plain Qt Widgets in the native style.
class EntryBrowserWidget : public QWidget
{
    Q_OBJECT

public:
    explicit EntryBrowserWidget(QWidget *parent = nullptr);

    void showEmpty();
    // scopeName identifies what is being loaded, e.g. "eoe.sw.unix";
    // it is kept for the table header once the data arrives.
    void showLoading(const QString &scopeName);
    // Shows the entries. When the snapshot is invalid (duplicate
    // entry keys) the error page is shown instead and nothing is
    // replaced.
    void showEntries(const EntryListSnapshot &entries);
    void showError(const QString &message);

signals:
    // Emitted only for explicit row picks. A model reset that drops
    // the selection emits nothing, so reloading the files never
    // clobbers the hierarchy detail the inspector is showing.
    void entrySelected(quint64 productId, quint64 entryId);

private:
    void onCurrentRowChanged(const QModelIndex &current, const QModelIndex &previous);

    QStackedWidget *m_stack = nullptr;
    QWidget *m_emptyPage = nullptr;
    QWidget *m_loadingPage = nullptr;
    QWidget *m_tablePage = nullptr;
    QWidget *m_errorPage = nullptr;
    QLabel *m_errorMessage = nullptr;

    QLabel *m_scopeLabel = nullptr;
    QLabel *m_countLabel = nullptr;
    QTableView *m_tableView = nullptr;
    EntryTableModel *m_model = nullptr;
};
