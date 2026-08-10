#include "ExtractionDialog.h"

#include <QCheckBox>
#include <QLabel>
#include <QLineEdit>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QRadioButton>
#include <QSignalSpy>
#include <QStackedWidget>
#include <QTest>

// Drives ExtractionDialog directly (no backend): defaults, validation,
// the preflight/plan/extract state machine, invalidation, and the
// rendering of plans, reports, failures and recoveries.
class ExtractionDialogTest : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void defaultsMatchTheSpec();
    void selectedEntryRadioFollowsScope();
    void relativeToControlsFollowTheRadio();
    void storedModeDisablesKeepStored();
    void emptyOutputDirIsRejected();
    void relativeOutputDirIsRejected();
    void nonexistentAbsoluteOutputDirIsAccepted();
    void hardwareProfileRendersVerbatim();
    void preflightEmitsTheAssembledRequest();
    void planReadyShowsSummaryAndEnablesExtract();
    void planFailureKeepsOptionsWithoutMessageBox();
    void anyOptionChangeInvalidatesThePlan();
    void optionChangeDuringCheckInvalidates();
    void lateResponsesAreDropped();
    void overwriteCountShowsWarning();
    void runningBlocksCloseAndReject();
    void reportRendersCountsFailuresAndRecoveries();
    void mediaTextStaysPlain();
};

namespace {

ExtractionPlanSnapshot makePlan(quint64 existingOutputs = 0)
{
    ExtractionPlanSnapshot plan;
    plan.requestedRecords = 3;
    plan.omittedRecords = 1;
    plan.hardwareExcludedRecords = 2;
    plan.plannedRecords = 4;
    plan.outputPaths = 5;
    plan.existingOutputs = existingOutputs;
    return plan;
}

QWidget *currentPage(ExtractionDialog &dialog)
{
    return dialog.findChild<QStackedWidget *>()->currentWidget();
}

QString currentPageName(ExtractionDialog &dialog)
{
    return currentPage(dialog)->objectName();
}

} // namespace

void ExtractionDialogTest::initTestCase()
{
    qRegisterMetaType<ExtractionRequestSnapshot>("ExtractionRequestSnapshot");
}

void ExtractionDialogTest::defaultsMatchTheSpec()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}, {1, 1}, {2, 0}}, EntryKey{1, 1}, QStringLiteral("usr/lib/libGL.so"));

    auto *currentView = dialog.findChild<QRadioButton *>(QStringLiteral("currentViewRadio"));
    auto *selected = dialog.findChild<QRadioButton *>(QStringLiteral("selectedEntryRadio"));
    QVERIFY(currentView->isChecked());
    QCOMPARE(currentView->text(), QStringLiteral("Current Files view (3 records)"));
    QVERIFY(selected->isEnabled());
    QCOMPARE(selected->text(), QStringLiteral("Selected entry (usr/lib/libGL.so)"));

    QVERIFY(dialog.findChild<QRadioButton *>(QStringLiteral("fullPathsRadio"))->isChecked());
    QVERIFY(dialog.findChild<QRadioButton *>(QStringLiteral("decodeRadio"))->isChecked());
    QVERIFY(!dialog.findChild<QCheckBox *>(QStringLiteral("allowOverwriteCheck"))->isChecked());
    QVERIFY(dialog.findChild<QCheckBox *>(QStringLiteral("continueOnErrorCheck"))->isChecked());
    QVERIFY(!dialog.findChild<QCheckBox *>(QStringLiteral("keepStoredCheck"))->isChecked());
    QVERIFY(dialog.findChild<QCheckBox *>(QStringLiteral("keepStoredCheck"))->isEnabled());
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    QCOMPARE(currentPageName(dialog), QStringLiteral("setupPage"));
}

void ExtractionDialogTest::selectedEntryRadioFollowsScope()
{
    ExtractionDialog withSelection;
    withSelection.setScope({{1, 0}}, EntryKey{1, 0}, QStringLiteral("a.txt"));
    QVERIFY(withSelection.findChild<QRadioButton *>(QStringLiteral("selectedEntryRadio"))
                ->isEnabled());

    ExtractionDialog withoutSelection;
    withoutSelection.setScope({{1, 0}, {1, 1}}, std::nullopt, QString());
    auto *selected = withoutSelection.findChild<QRadioButton *>(QStringLiteral("selectedEntryRadio"));
    QVERIFY(!selected->isEnabled());
    QVERIFY(withoutSelection.findChild<QRadioButton *>(QStringLiteral("currentViewRadio"))
                ->isChecked());
}

void ExtractionDialogTest::relativeToControlsFollowTheRadio()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());

    auto *edit = dialog.findChild<QLineEdit *>(QStringLiteral("relativeToEdit"));
    QVERIFY(!edit->isEnabled());
    dialog.findChild<QRadioButton *>(QStringLiteral("relativeToRadio"))->setChecked(true);
    QVERIFY(edit->isEnabled());
    dialog.findChild<QRadioButton *>(QStringLiteral("fullPathsRadio"))->setChecked(true);
    QVERIFY(!edit->isEnabled());
}

void ExtractionDialogTest::storedModeDisablesKeepStored()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());

    auto *keepStored = dialog.findChild<QCheckBox *>(QStringLiteral("keepStoredCheck"));
    keepStored->setChecked(true);
    QVERIFY(keepStored->isChecked());

    // The stored mode has no decoded file to keep a sidecar for.
    dialog.findChild<QRadioButton *>(QStringLiteral("storedRadio"))->setChecked(true);
    QVERIFY(!keepStored->isEnabled());
    QVERIFY(!keepStored->isChecked());

    dialog.findChild<QRadioButton *>(QStringLiteral("decodeRadio"))->setChecked(true);
    QVERIFY(keepStored->isEnabled());
}

void ExtractionDialogTest::emptyOutputDirIsRejected()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());

    QSignalSpy spy(&dialog, &ExtractionDialog::preflightRequested);
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(spy.count(), 0);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    QVERIFY(!dialog.findChild<QLabel *>(QStringLiteral("statusLabel"))->text().isEmpty());
}

void ExtractionDialogTest::relativeOutputDirIsRejected()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("relative/out"));

    QSignalSpy spy(&dialog, &ExtractionDialog::preflightRequested);
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(spy.count(), 0);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    QCOMPARE(dialog.findChild<QLabel *>(QStringLiteral("statusLabel"))->text(),
             QStringLiteral("The output directory must be an absolute path."));
}

void ExtractionDialogTest::nonexistentAbsoluteOutputDirIsAccepted()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-not-existing"));

    QSignalSpy spy(&dialog, &ExtractionDialog::preflightRequested);
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(spy.count(), 1);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Checking);
    // The double-click guard: no second request while one is in
    // flight.
    QVERIFY(!dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->isEnabled());
}

void ExtractionDialogTest::hardwareProfileRendersVerbatim()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());

    auto *summary = dialog.findChild<QLabel *>(QStringLiteral("hardwareSummaryLabel"));
    auto *hint = dialog.findChild<QLabel *>(QStringLiteral("hardwareHintLabel"));

    dialog.setHardwareProfile({});
    QCOMPARE(summary->text(),
             QStringLiteral("Hardware: none\nNo hardware profile will be applied."));
    QVERIFY(hint->isHidden());

    // An empty value is a genuine fact: `GFXBOARD=` verbatim, never a
    // placeholder.
    dialog.setHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                               {QStringLiteral("CPUARCH"), QStringLiteral("R4400")},
                               {QStringLiteral("GFXBOARD"), QString()}});
    QCOMPARE(summary->text(),
             QStringLiteral("CPUBOARD=IP22\nCPUARCH=R4400\nGFXBOARD="));
    QVERIFY(!hint->isHidden());
}

void ExtractionDialogTest::preflightEmitsTheAssembledRequest()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}, {1, 1}, {2, 0}}, EntryKey{1, 1}, QStringLiteral("usr/lib/libGL.so"));
    dialog.setHardwareProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")},
                               {QStringLiteral("GFXBOARD"), QString()}});
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-request"));

    QSignalSpy spy(&dialog, &ExtractionDialog::preflightRequested);
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(spy.count(), 1);

    const auto request = qvariant_cast<ExtractionRequestSnapshot>(spy.first().at(0));
    QCOMPARE(request.entries, (QList<EntryKey>{{1, 0}, {1, 1}, {2, 0}}));
    QCOMPARE(request.hardware.size(), 2);
    QCOMPARE(request.hardware.at(1).attribute, QStringLiteral("GFXBOARD"));
    // The empty value reaches the backend as an empty value.
    QCOMPARE(request.hardware.at(1).value, QString());
    QCOMPARE(request.outputDir, QStringLiteral("/tmp/sw-explorer-dialog-test-request"));
    QCOMPARE(request.pathMode, ExtractionPathMode::Full);
    QCOMPARE(request.decode, ExtractionDecodeMode::Auto);
    QVERIFY(!request.keepStored);
    QVERIFY(request.continueOnError);
    QVERIFY(!request.allowOverwrite);
}

void ExtractionDialogTest::planReadyShowsSummaryAndEnablesExtract()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-plan"));
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(dialog.state(), ExtractionDialog::State::Checking);

    dialog.showPlan(makePlan());
    QCOMPARE(dialog.state(), ExtractionDialog::State::PlanReady);
    QCOMPARE(currentPageName(dialog), QStringLiteral("planPage"));

    auto *body = dialog.findChild<QLabel *>(QStringLiteral("planBodyLabel"));
    QCOMPARE(body->textFormat(), Qt::PlainText);
    QVERIFY(body->text().contains(QStringLiteral("Requested records:          3")));
    QVERIFY(body->text().contains(QStringLiteral("Omitted records:            1")));
    QVERIFY(body->text().contains(QStringLiteral("Hardware-excluded records:  2")));
    QVERIFY(body->text().contains(QStringLiteral("Planned records:            4")));
    QVERIFY(body->text().contains(QStringLiteral("Output paths:               5")));
    QVERIFY(body->text().contains(QStringLiteral("Existing outputs:           0")));
    QVERIFY(dialog.findChild<QLabel *>(QStringLiteral("overwriteWarningLabel"))
                ->text()
                .isEmpty());
}

void ExtractionDialogTest::planFailureKeepsOptionsWithoutMessageBox()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-refused"));
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();

    dialog.showPlanFailure(QStringLiteral("extraction is ambiguous\n\n2 hardware conflicts affect "
                                          "this scope:\n\nusr/share/Insight/lib"));
    QCOMPARE(dialog.state(), ExtractionDialog::State::Failed);
    QCOMPARE(currentPageName(dialog), QStringLiteral("failurePage"));
    QCOMPARE(dialog.findChild<QLabel *>(QStringLiteral("failureTitleLabel"))->text(),
             QStringLiteral("Extraction refused"));
    QVERIFY(dialog.findChild<QPlainTextEdit *>(QStringLiteral("failureMessageEdit"))
                ->toPlainText()
                .contains(QStringLiteral("usr/share/Insight/lib")));
    QCOMPARE(dialog.findChild<QLabel *>(QStringLiteral("failureNoteLabel"))->text(),
             QStringLiteral("No files were written."));

    // The options survived; Back returns to them for another try.
    dialog.findChild<QPushButton *>(QStringLiteral("failureBackButton"))->click();
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    QCOMPARE(currentPageName(dialog), QStringLiteral("setupPage"));
    QCOMPARE(dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))->text(),
             QStringLiteral("/tmp/sw-explorer-dialog-test-refused"));
}

void ExtractionDialogTest::anyOptionChangeInvalidatesThePlan()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-invalidate"));
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    dialog.showPlan(makePlan());
    QCOMPARE(dialog.state(), ExtractionDialog::State::PlanReady);

    QSignalSpy spy(&dialog, &ExtractionDialog::requestInvalidated);
    dialog.findChild<QCheckBox *>(QStringLiteral("keepStoredCheck"))->setChecked(true);
    QCOMPARE(spy.count(), 1);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    QCOMPARE(currentPageName(dialog), QStringLiteral("setupPage"));

    // Extract is unreachable without a fresh preflight: a new
    // preflight must happen first.
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(dialog.state(), ExtractionDialog::State::Checking);
    dialog.showPlan(makePlan());
    QCOMPARE(dialog.state(), ExtractionDialog::State::PlanReady);
}

void ExtractionDialogTest::optionChangeDuringCheckInvalidates()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-inflight"));
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    QCOMPARE(dialog.state(), ExtractionDialog::State::Checking);

    QSignalSpy spy(&dialog, &ExtractionDialog::requestInvalidated);
    dialog.findChild<QCheckBox *>(QStringLiteral("allowOverwriteCheck"))->setChecked(true);
    QCOMPARE(spy.count(), 1);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    // A late preflight answer for the abandoned request is dropped.
    dialog.showPlan(makePlan());
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
}

void ExtractionDialogTest::lateResponsesAreDropped()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());

    // Nothing was ever requested: a response cannot appear out of
    // nowhere.
    dialog.showPlan(makePlan());
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    dialog.showPlanFailure(QStringLiteral("late"));
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    dialog.showReport({});
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
    dialog.showExtractionFailure(QStringLiteral("late"));
    QCOMPARE(dialog.state(), ExtractionDialog::State::Setup);
}

void ExtractionDialogTest::overwriteCountShowsWarning()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-warn"));
    dialog.findChild<QCheckBox *>(QStringLiteral("allowOverwriteCheck"))->setChecked(true);
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();

    dialog.showPlan(makePlan(3));
    QCOMPARE(dialog.state(), ExtractionDialog::State::PlanReady);
    // A warning, never an error: Extract stays available.
    QCOMPARE(dialog.findChild<QLabel *>(QStringLiteral("overwriteWarningLabel"))->text(),
             QStringLiteral("⚠ 3 existing regular files will be overwritten."));
}

void ExtractionDialogTest::runningBlocksCloseAndReject()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-running"));
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    dialog.showPlan(makePlan());

    QSignalSpy spy(&dialog, &ExtractionDialog::extractRequested);
    dialog.show();
    dialog.findChild<QPushButton *>(QStringLiteral("extractButton"))->click();
    QCOMPARE(spy.count(), 1);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Running);
    QCOMPARE(currentPageName(dialog), QStringLiteral("runningPage"));

    // No cancel: reject, Esc and the window X are all blocked.
    dialog.reject();
    QCOMPARE(dialog.state(), ExtractionDialog::State::Running);
    QVERIFY(dialog.isVisible());

    // Once the batch completes, closing works again.
    dialog.showReport({});
    QCOMPARE(dialog.state(), ExtractionDialog::State::Done);
    dialog.reject();
    QVERIFY(!dialog.isVisible());
}

void ExtractionDialogTest::reportRendersCountsFailuresAndRecoveries()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    dialog.findChild<QLineEdit *>(QStringLiteral("outputDirEdit"))
        ->setText(QStringLiteral("/tmp/sw-explorer-dialog-test-report"));
    dialog.findChild<QPushButton *>(QStringLiteral("preflightButton"))->click();
    dialog.showPlan(makePlan());
    dialog.findChild<QPushButton *>(QStringLiteral("extractButton"))->click();

    ExtractionReportSnapshot report;
    report.extracted = 2659;
    report.skipped = 0;
    report.failures = {{QStringLiteral("usr/bin/bad"), QStringLiteral("I/O error")}};
    report.recoveries = {{QStringLiteral("usr/lib/foo"),
                          true,
                          0x1234,
                          0x1240,
                          ExtractionRecoveryKind::Delta,
                          true,
                          12},
                         {QStringLiteral("usr/lib/bar"),
                          false,
                          0,
                          0x2000,
                          ExtractionRecoveryKind::Resynced,
                          false,
                          0}};
    dialog.showReport(report);
    QCOMPARE(dialog.state(), ExtractionDialog::State::Done);
    QCOMPARE(currentPageName(dialog), QStringLiteral("resultPage"));

    QCOMPARE(dialog.findChild<QLabel *>(QStringLiteral("resultTitleLabel"))->text(),
             QStringLiteral("Extraction completed with errors"));
    const QString body =
        dialog.findChild<QPlainTextEdit *>(QStringLiteral("resultBodyEdit"))->toPlainText();
    QVERIFY(body.contains(QStringLiteral("Extracted:    2659")));
    QVERIFY(body.contains(QStringLiteral("Skipped:      0")));
    QVERIFY(body.contains(QStringLiteral("Failed:       1")));
    QVERIFY(body.contains(QStringLiteral("Recovered:    2")));
    QVERIFY(body.contains(QStringLiteral("usr/bin/bad: I/O error")));
    QVERIFY(body.contains(QStringLiteral("usr/lib/foo")));
    QVERIFY(body.contains(QStringLiteral("expected:   0x1234")));
    QVERIFY(body.contains(QStringLiteral("actual:     0x1240")));
    QVERIFY(body.contains(QStringLiteral("resolution: Delta +12")));
    QVERIFY(body.contains(QStringLiteral("resolution: Resynced")));
}

void ExtractionDialogTest::mediaTextStaysPlain()
{
    ExtractionDialog dialog;
    dialog.setScope({{1, 0}}, std::nullopt, QString());
    for (const char *name :
         {"statusLabel", "planBodyLabel", "overwriteWarningLabel", "hardwareSummaryLabel"}) {
        auto *label = dialog.findChild<QLabel *>(QString::fromLatin1(name));
        QVERIFY2(label != nullptr, name);
        QCOMPARE(label->textFormat(), Qt::PlainText);
    }
    // Paths and messages never become rich text anywhere else either:
    // the long-form renderers are plain-text edits by construction.
    QVERIFY(dialog.findChild<QPlainTextEdit *>(QStringLiteral("resultBodyEdit")) != nullptr);
    QVERIFY(dialog.findChild<QPlainTextEdit *>(QStringLiteral("failureMessageEdit")) != nullptr);
}

QTEST_MAIN(ExtractionDialogTest)

#include "ExtractionDialogTest.moc"
