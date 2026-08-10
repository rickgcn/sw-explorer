#pragma once

#include "ExtractionSnapshot.h"

#include <QDialog>

#include <optional>

QT_BEGIN_NAMESPACE
class QCheckBox;
class QLabel;
class QLineEdit;
class QPlainTextEdit;
class QProgressBar;
class QPushButton;
class QRadioButton;
class QStackedWidget;
QT_END_NAMESPACE

// The extraction dialog: scope and options on one page, an async
// preflight against the backend, then — only after a successful
// preflight — the actual extraction, which re-plans inside the
// backend before anything is written.
//
// The dialog never talks to the worker itself: it emits complete
// ExtractionRequestSnapshot values and the MainWindow routes them
// through the extraction request family. Any option change after a
// preflight invalidates it (requestInvalidated), forcing a fresh
// preflight before Extract is available again.
//
// There is deliberately no cancellation: the core batch runs
// synchronously on the worker thread, so while it runs the dialog
// cannot be closed, rejected or escaped.
class ExtractionDialog : public QDialog
{
    Q_OBJECT

public:
    // The dialog's own state machine.
    enum class State {
        // Editing options; Preflight available.
        Setup,
        // A preflight request is in flight.
        Checking,
        // A plan is confirmed; Extract available.
        PlanReady,
        // The extraction is running; the dialog cannot be closed.
        Running,
        // The extraction finished; the report is shown.
        Done,
        // The planner refused, at preflight or at execution time.
        Failed,
    };

    explicit ExtractionDialog(QWidget *parent = nullptr);

    // The two scopes the user chooses between: every row of the
    // current Files view, or the currently selected row.
    void setScope(const QList<EntryKey> &viewKeys,
                  const std::optional<EntryKey> &selectedKey,
                  const QString &selectedPath);
    // The applied hardware profile, rendered verbatim (empty values
    // included) and passed through to every request unchanged.
    void setHardwareProfile(const HardwareProfileSnapshot &profile);

    State state() const;

    // MainWindow forwards the worker responses through these.
    void showPlan(const ExtractionPlanSnapshot &plan);
    void showPlanFailure(const QString &message);
    void showReport(const ExtractionReportSnapshot &report);
    void showExtractionFailure(const QString &message);

    void reject() override;

signals:
    void preflightRequested(const ExtractionRequestSnapshot &request);
    void extractRequested(const ExtractionRequestSnapshot &request);
    // The request changed after (or during) a preflight, or the
    // dialog was closed mid-check: anything in flight is stale.
    void requestInvalidated();

private slots:
    void onOptionsChanged();
    void onBrowse();
    void onPreflight();
    void onExtract();
    void onBackToSetup();
    void onFailureBack();

private:
    // Assembles the current request from the widgets; validates the
    // output directory and the RelativeTo prefix locally (the Rust
    // side remains the grammar authority) and reports problems on the
    // status label. Returns nullopt when invalid.
    std::optional<ExtractionRequestSnapshot> validatedRequest();
    // The current request without validation.
    ExtractionRequestSnapshot buildRequest() const;
    void showFailurePage(const QString &title, const QString &message, const QString &note);

    State m_state = State::Setup;

    QList<EntryKey> m_viewKeys;
    std::optional<EntryKey> m_selectedKey;
    HardwareProfileSnapshot m_profile;

    QStackedWidget *m_stack = nullptr;
    QWidget *m_setupPage = nullptr;
    QWidget *m_planPage = nullptr;
    QWidget *m_runningPage = nullptr;
    QWidget *m_resultPage = nullptr;
    QWidget *m_failurePage = nullptr;

    QRadioButton *m_currentViewRadio = nullptr;
    QRadioButton *m_selectedEntryRadio = nullptr;
    QLabel *m_hardwareSummaryLabel = nullptr;
    QLabel *m_hardwareHintLabel = nullptr;
    QLineEdit *m_outputDirEdit = nullptr;
    QPushButton *m_browseButton = nullptr;
    QRadioButton *m_fullPathsRadio = nullptr;
    QRadioButton *m_flatRadio = nullptr;
    QRadioButton *m_relativeToRadio = nullptr;
    QLineEdit *m_relativeToEdit = nullptr;
    QRadioButton *m_decodeRadio = nullptr;
    QRadioButton *m_storedRadio = nullptr;
    QCheckBox *m_keepStoredCheck = nullptr;
    QCheckBox *m_continueOnErrorCheck = nullptr;
    QCheckBox *m_allowOverwriteCheck = nullptr;
    QLabel *m_statusLabel = nullptr;
    QPushButton *m_cancelButton = nullptr;
    QPushButton *m_preflightButton = nullptr;

    QLabel *m_planBodyLabel = nullptr;
    QLabel *m_overwriteWarningLabel = nullptr;
    QPushButton *m_planBackButton = nullptr;
    QPushButton *m_extractButton = nullptr;

    QProgressBar *m_runningProgress = nullptr;

    QLabel *m_resultTitleLabel = nullptr;
    QPlainTextEdit *m_resultBodyEdit = nullptr;
    QPushButton *m_resultCloseButton = nullptr;

    QLabel *m_failureTitleLabel = nullptr;
    QPlainTextEdit *m_failureMessageEdit = nullptr;
    QLabel *m_failureNoteLabel = nullptr;
    QPushButton *m_failureBackButton = nullptr;
    QPushButton *m_failureCloseButton = nullptr;
};
