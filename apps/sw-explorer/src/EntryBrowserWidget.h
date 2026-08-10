#pragma once

#include "EntrySnapshot.h"
#include "HardwareSnapshot.h"

#include <QWidget>

#include <optional>

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
//
// The hardware selection is an overlay over the table: while a
// selection is applied the Status column is visible and the footer
// counts selected and conflicted records. Applying or clearing the
// overlay never reloads, reorders or filters the rows.
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

    // Overlays a hardware selection: the Status column becomes
    // visible and the footer counts the selected and conflicted
    // records among the current rows. The rows themselves are
    // untouched.
    void setSelectionOverlay(const SelectionSnapshot &selection);
    // Clears the selection overlay and hides the Status column.
    void clearSelectionOverlay();

    // The extraction scope of the current view: the keys of every row
    // being displayed, in exact display order. Empty when no rows are
    // visible (Empty / Loading / Error page, or a table without
    // rows): a stale model must never feed an extraction.
    QList<EntryKey> entryKeys() const;
    // The key of the currently selected row, if one is visible.
    std::optional<EntryKey> currentEntryKey() const;
    // The path of the currently selected row, for scope labels;
    // empty when nothing is selected.
    QString currentEntryPath() const;
    // Whether the table page is showing at least one row right now.
    bool hasVisibleEntries() const;

signals:
    // Emitted only for explicit row picks. A model reset that drops
    // the selection emits nothing, so reloading the files never
    // clobbers the hierarchy detail the inspector is showing.
    void entrySelected(quint64 productId, quint64 entryId);

private:
    void onCurrentRowChanged(const QModelIndex &current, const QModelIndex &previous);
    // Rebuilds the footer from the current rows and overlay state.
    void updateFooter();

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
