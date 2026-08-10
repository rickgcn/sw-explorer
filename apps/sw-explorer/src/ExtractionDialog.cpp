#include "ExtractionDialog.h"

#include <QButtonGroup>
#include <QCheckBox>
#include <QDir>
#include <QFileDialog>
#include <QFormLayout>
#include <QGroupBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QStackedWidget>
#include <QVBoxLayout>

ExtractionDialog::ExtractionDialog(QWidget *parent)
    : QDialog(parent)
{
    setWindowTitle(tr("Extract"));
    resize(560, 640);

    auto *layout = new QVBoxLayout(this);
    m_stack = new QStackedWidget(this);
    layout->addWidget(m_stack);

    // ---- Setup page: scope, hardware, output and options. ----
    m_setupPage = new QWidget(this);
    m_setupPage->setObjectName(QStringLiteral("setupPage"));
    auto *setupLayout = new QVBoxLayout(m_setupPage);

    auto *scopeGroup = new QGroupBox(tr("Scope"), m_setupPage);
    auto *scopeLayout = new QVBoxLayout(scopeGroup);
    m_currentViewRadio = new QRadioButton(scopeGroup);
    m_currentViewRadio->setObjectName(QStringLiteral("currentViewRadio"));
    m_currentViewRadio->setChecked(true);
    m_selectedEntryRadio = new QRadioButton(scopeGroup);
    m_selectedEntryRadio->setObjectName(QStringLiteral("selectedEntryRadio"));
    scopeLayout->addWidget(m_currentViewRadio);
    scopeLayout->addWidget(m_selectedEntryRadio);
    setupLayout->addWidget(scopeGroup);

    auto *hardwareGroup = new QGroupBox(tr("Hardware"), m_setupPage);
    auto *hardwareLayout = new QVBoxLayout(hardwareGroup);
    m_hardwareSummaryLabel = new QLabel(hardwareGroup);
    m_hardwareSummaryLabel->setObjectName(QStringLiteral("hardwareSummaryLabel"));
    m_hardwareSummaryLabel->setTextFormat(Qt::PlainText);
    m_hardwareSummaryLabel->setTextInteractionFlags(Qt::TextSelectableByMouse);
    m_hardwareHintLabel = new QLabel(tr("Selection will be re-evaluated before extraction."),
                                     hardwareGroup);
    m_hardwareHintLabel->setObjectName(QStringLiteral("hardwareHintLabel"));
    hardwareLayout->addWidget(m_hardwareSummaryLabel);
    hardwareLayout->addWidget(m_hardwareHintLabel);
    setupLayout->addWidget(hardwareGroup);

    auto *outputGroup = new QGroupBox(tr("Output"), m_setupPage);
    auto *outputLayout = new QHBoxLayout(outputGroup);
    m_outputDirEdit = new QLineEdit(outputGroup);
    m_outputDirEdit->setObjectName(QStringLiteral("outputDirEdit"));
    m_outputDirEdit->setPlaceholderText(tr("/absolute/output/directory"));
    m_browseButton = new QPushButton(tr("Browse..."), outputGroup);
    m_browseButton->setObjectName(QStringLiteral("browseButton"));
    outputLayout->addWidget(m_outputDirEdit, 1);
    outputLayout->addWidget(m_browseButton);
    setupLayout->addWidget(outputGroup);

    auto *pathsGroup = new QGroupBox(tr("Paths"), m_setupPage);
    auto *pathsLayout = new QVBoxLayout(pathsGroup);
    m_fullPathsRadio = new QRadioButton(tr("Full paths"), pathsGroup);
    m_fullPathsRadio->setObjectName(QStringLiteral("fullPathsRadio"));
    m_fullPathsRadio->setChecked(true);
    m_flatRadio = new QRadioButton(tr("Flat"), pathsGroup);
    m_flatRadio->setObjectName(QStringLiteral("flatRadio"));
    auto *relativeRow = new QWidget(pathsGroup);
    auto *relativeLayout = new QHBoxLayout(relativeRow);
    relativeLayout->setContentsMargins(0, 0, 0, 0);
    m_relativeToRadio = new QRadioButton(tr("Relative to:"), relativeRow);
    m_relativeToRadio->setObjectName(QStringLiteral("relativeToRadio"));
    m_relativeToEdit = new QLineEdit(relativeRow);
    m_relativeToEdit->setObjectName(QStringLiteral("relativeToEdit"));
    m_relativeToEdit->setPlaceholderText(tr("usr/..."));
    m_relativeToEdit->setEnabled(false);
    relativeLayout->addWidget(m_relativeToRadio);
    relativeLayout->addWidget(m_relativeToEdit, 1);
    pathsLayout->addWidget(m_fullPathsRadio);
    pathsLayout->addWidget(m_flatRadio);
    pathsLayout->addWidget(relativeRow);
    setupLayout->addWidget(pathsGroup);

    // Auto-exclusion follows widget parents, and the RelativeTo radio
    // lives in a row widget of its own: make exclusivity explicit.
    auto *pathModeButtons = new QButtonGroup(this);
    pathModeButtons->addButton(m_fullPathsRadio);
    pathModeButtons->addButton(m_flatRadio);
    pathModeButtons->addButton(m_relativeToRadio);

    auto *payloadGroup = new QGroupBox(tr("Payload"), m_setupPage);
    auto *payloadLayout = new QVBoxLayout(payloadGroup);
    m_decodeRadio = new QRadioButton(tr("Decode compressed files"), payloadGroup);
    m_decodeRadio->setObjectName(QStringLiteral("decodeRadio"));
    m_decodeRadio->setChecked(true);
    m_storedRadio = new QRadioButton(tr("Write payloads as stored"), payloadGroup);
    m_storedRadio->setObjectName(QStringLiteral("storedRadio"));
    m_keepStoredCheck = new QCheckBox(tr("Also keep compressed .Z copy"), payloadGroup);
    m_keepStoredCheck->setObjectName(QStringLiteral("keepStoredCheck"));
    payloadLayout->addWidget(m_decodeRadio);
    payloadLayout->addWidget(m_storedRadio);
    payloadLayout->addWidget(m_keepStoredCheck);
    setupLayout->addWidget(payloadGroup);

    auto *errorsGroup = new QGroupBox(tr("Errors"), m_setupPage);
    auto *errorsLayout = new QVBoxLayout(errorsGroup);
    m_continueOnErrorCheck = new QCheckBox(tr("Continue after individual extraction errors"),
                                           errorsGroup);
    m_continueOnErrorCheck->setObjectName(QStringLiteral("continueOnErrorCheck"));
    m_continueOnErrorCheck->setChecked(true);
    errorsLayout->addWidget(m_continueOnErrorCheck);
    setupLayout->addWidget(errorsGroup);

    auto *existingGroup = new QGroupBox(tr("Existing files"), m_setupPage);
    auto *existingLayout = new QVBoxLayout(existingGroup);
    m_allowOverwriteCheck =
        new QCheckBox(tr("Allow overwriting existing regular files"), existingGroup);
    m_allowOverwriteCheck->setObjectName(QStringLiteral("allowOverwriteCheck"));
    existingLayout->addWidget(m_allowOverwriteCheck);
    setupLayout->addWidget(existingGroup);

    m_statusLabel = new QLabel(m_setupPage);
    m_statusLabel->setObjectName(QStringLiteral("statusLabel"));
    m_statusLabel->setTextFormat(Qt::PlainText);
    m_statusLabel->setWordWrap(true);
    setupLayout->addWidget(m_statusLabel);

    setupLayout->addStretch();

    auto *setupButtons = new QHBoxLayout;
    setupButtons->addStretch();
    m_cancelButton = new QPushButton(tr("Cancel"), m_setupPage);
    m_cancelButton->setObjectName(QStringLiteral("cancelButton"));
    m_preflightButton = new QPushButton(tr("Preflight"), m_setupPage);
    m_preflightButton->setObjectName(QStringLiteral("preflightButton"));
    m_preflightButton->setDefault(true);
    setupButtons->addWidget(m_cancelButton);
    setupButtons->addWidget(m_preflightButton);
    setupLayout->addLayout(setupButtons);

    // ---- Plan page: the confirmed plan. ----
    m_planPage = new QWidget(this);
    m_planPage->setObjectName(QStringLiteral("planPage"));
    auto *planLayout = new QVBoxLayout(m_planPage);

    auto *planTitle = new QLabel(tr("Ready to extract"), m_planPage);
    QFont planTitleFont = planTitle->font();
    planTitleFont.setBold(true);
    planTitle->setFont(planTitleFont);
    planLayout->addWidget(planTitle);

    m_planBodyLabel = new QLabel(m_planPage);
    m_planBodyLabel->setObjectName(QStringLiteral("planBodyLabel"));
    m_planBodyLabel->setTextFormat(Qt::PlainText);
    m_planBodyLabel->setTextInteractionFlags(Qt::TextSelectableByMouse);
    planLayout->addWidget(m_planBodyLabel);

    m_overwriteWarningLabel = new QLabel(m_planPage);
    m_overwriteWarningLabel->setObjectName(QStringLiteral("overwriteWarningLabel"));
    m_overwriteWarningLabel->setTextFormat(Qt::PlainText);
    m_overwriteWarningLabel->setWordWrap(true);
    planLayout->addWidget(m_overwriteWarningLabel);

    auto *safeLabel = new QLabel(tr("✓ Safe to extract"), m_planPage);
    safeLabel->setObjectName(QStringLiteral("planSafeLabel"));
    planLayout->addWidget(safeLabel);

    planLayout->addStretch();

    auto *planButtons = new QHBoxLayout;
    planButtons->addStretch();
    m_planBackButton = new QPushButton(tr("Back"), m_planPage);
    m_planBackButton->setObjectName(QStringLiteral("planBackButton"));
    m_extractButton = new QPushButton(tr("Extract"), m_planPage);
    m_extractButton->setObjectName(QStringLiteral("extractButton"));
    m_extractButton->setDefault(true);
    planButtons->addWidget(m_planBackButton);
    planButtons->addWidget(m_extractButton);
    planLayout->addLayout(planButtons);

    // ---- Running page: an honest indeterminate wait. ----
    m_runningPage = new QWidget(this);
    m_runningPage->setObjectName(QStringLiteral("runningPage"));
    auto *runningLayout = new QVBoxLayout(m_runningPage);
    runningLayout->addStretch();
    auto *runningLabel = new QLabel(tr("Extracting..."), m_runningPage);
    runningLabel->setAlignment(Qt::AlignCenter);
    runningLayout->addWidget(runningLabel);
    m_runningProgress = new QProgressBar(m_runningPage);
    m_runningProgress->setObjectName(QStringLiteral("runningProgress"));
    // No percentage is known; the batch runs synchronously.
    m_runningProgress->setRange(0, 0);
    runningLayout->addWidget(m_runningProgress);
    runningLayout->addStretch();

    // ---- Result page: the extraction report. ----
    m_resultPage = new QWidget(this);
    m_resultPage->setObjectName(QStringLiteral("resultPage"));
    auto *resultLayout = new QVBoxLayout(m_resultPage);

    m_resultTitleLabel = new QLabel(m_resultPage);
    m_resultTitleLabel->setObjectName(QStringLiteral("resultTitleLabel"));
    QFont resultTitleFont = m_resultTitleLabel->font();
    resultTitleFont.setBold(true);
    m_resultTitleLabel->setFont(resultTitleFont);
    resultLayout->addWidget(m_resultTitleLabel);

    m_resultBodyEdit = new QPlainTextEdit(m_resultPage);
    m_resultBodyEdit->setObjectName(QStringLiteral("resultBodyEdit"));
    m_resultBodyEdit->setReadOnly(true);
    resultLayout->addWidget(m_resultBodyEdit, 1);

    auto *resultButtons = new QHBoxLayout;
    resultButtons->addStretch();
    m_resultCloseButton = new QPushButton(tr("Close"), m_resultPage);
    m_resultCloseButton->setObjectName(QStringLiteral("resultCloseButton"));
    resultButtons->addWidget(m_resultCloseButton);
    resultLayout->addLayout(resultButtons);

    // ---- Failure page: a planner refusal, options preserved. ----
    m_failurePage = new QWidget(this);
    m_failurePage->setObjectName(QStringLiteral("failurePage"));
    auto *failureLayout = new QVBoxLayout(m_failurePage);

    m_failureTitleLabel = new QLabel(m_failurePage);
    m_failureTitleLabel->setObjectName(QStringLiteral("failureTitleLabel"));
    QFont failureTitleFont = m_failureTitleLabel->font();
    failureTitleFont.setBold(true);
    m_failureTitleLabel->setFont(failureTitleFont);
    failureLayout->addWidget(m_failureTitleLabel);

    m_failureMessageEdit = new QPlainTextEdit(m_failurePage);
    m_failureMessageEdit->setObjectName(QStringLiteral("failureMessageEdit"));
    m_failureMessageEdit->setReadOnly(true);
    failureLayout->addWidget(m_failureMessageEdit, 1);

    m_failureNoteLabel = new QLabel(m_failurePage);
    m_failureNoteLabel->setObjectName(QStringLiteral("failureNoteLabel"));
    m_failureNoteLabel->setTextFormat(Qt::PlainText);
    failureLayout->addWidget(m_failureNoteLabel);

    auto *failureButtons = new QHBoxLayout;
    failureButtons->addStretch();
    m_failureBackButton = new QPushButton(tr("Back"), m_failurePage);
    m_failureBackButton->setObjectName(QStringLiteral("failureBackButton"));
    m_failureCloseButton = new QPushButton(tr("Close"), m_failurePage);
    m_failureCloseButton->setObjectName(QStringLiteral("failureCloseButton"));
    failureButtons->addWidget(m_failureBackButton);
    failureButtons->addWidget(m_failureCloseButton);
    failureLayout->addLayout(failureButtons);

    m_stack->addWidget(m_setupPage);
    m_stack->addWidget(m_planPage);
    m_stack->addWidget(m_runningPage);
    m_stack->addWidget(m_resultPage);
    m_stack->addWidget(m_failurePage);
    m_stack->setCurrentWidget(m_setupPage);

    // Every option change funnels into the same invalidation point.
    connect(m_currentViewRadio, &QRadioButton::toggled, this, &ExtractionDialog::onOptionsChanged);
    connect(m_selectedEntryRadio,
            &QRadioButton::toggled,
            this,
            &ExtractionDialog::onOptionsChanged);
    connect(m_outputDirEdit, &QLineEdit::textChanged, this, &ExtractionDialog::onOptionsChanged);
    connect(m_fullPathsRadio, &QRadioButton::toggled, this, &ExtractionDialog::onOptionsChanged);
    connect(m_flatRadio, &QRadioButton::toggled, this, &ExtractionDialog::onOptionsChanged);
    connect(m_relativeToRadio, &QRadioButton::toggled, this, [this](bool checked) {
        m_relativeToEdit->setEnabled(checked);
        onOptionsChanged();
    });
    connect(m_relativeToEdit, &QLineEdit::textChanged, this, &ExtractionDialog::onOptionsChanged);
    connect(m_decodeRadio, &QRadioButton::toggled, this, [this](bool checked) {
        // keep_stored only exists with decoding; the stored mode
        // forces it off.
        m_keepStoredCheck->setEnabled(checked);
        if (!checked) {
            m_keepStoredCheck->setChecked(false);
        }
        onOptionsChanged();
    });
    connect(m_keepStoredCheck, &QCheckBox::toggled, this, &ExtractionDialog::onOptionsChanged);
    connect(m_continueOnErrorCheck,
            &QCheckBox::toggled,
            this,
            &ExtractionDialog::onOptionsChanged);
    connect(m_allowOverwriteCheck,
            &QCheckBox::toggled,
            this,
            &ExtractionDialog::onOptionsChanged);

    connect(m_browseButton, &QPushButton::clicked, this, &ExtractionDialog::onBrowse);
    connect(m_preflightButton, &QPushButton::clicked, this, &ExtractionDialog::onPreflight);
    connect(m_cancelButton, &QPushButton::clicked, this, &ExtractionDialog::reject);
    connect(m_extractButton, &QPushButton::clicked, this, &ExtractionDialog::onExtract);
    connect(m_planBackButton, &QPushButton::clicked, this, &ExtractionDialog::onBackToSetup);
    connect(m_resultCloseButton, &QPushButton::clicked, this, &ExtractionDialog::accept);
    connect(m_failureBackButton, &QPushButton::clicked, this, &ExtractionDialog::onFailureBack);
    connect(m_failureCloseButton, &QPushButton::clicked, this, &ExtractionDialog::reject);
}

void ExtractionDialog::setScope(const QList<EntryKey> &viewKeys,
                                const std::optional<EntryKey> &selectedKey,
                                const QString &selectedPath)
{
    m_viewKeys = viewKeys;
    m_selectedKey = selectedKey;
    m_currentViewRadio->setText(tr("Current Files view (%n records)", nullptr, viewKeys.size()));
    if (m_selectedKey.has_value()) {
        m_selectedEntryRadio->setText(tr("Selected entry (%1)").arg(selectedPath));
        m_selectedEntryRadio->setEnabled(true);
    } else {
        m_selectedEntryRadio->setText(tr("Selected entry (none)"));
        m_selectedEntryRadio->setEnabled(false);
        m_currentViewRadio->setChecked(true);
    }
}

void ExtractionDialog::setHardwareProfile(const HardwareProfileSnapshot &profile)
{
    m_profile = profile;
    if (m_profile.isEmpty()) {
        m_hardwareSummaryLabel->setText(
            tr("Hardware: none\nNo hardware profile will be applied."));
        m_hardwareHintLabel->setVisible(false);
        return;
    }
    // The pairs are rendered verbatim: an empty value stays an empty
    // value (`GFXBOARD=`), never a placeholder.
    QStringList lines;
    lines.reserve(m_profile.size());
    for (const HardwareValueSnapshot &pair : m_profile) {
        lines.append(QStringLiteral("%1=%2").arg(pair.attribute, pair.value));
    }
    m_hardwareSummaryLabel->setText(lines.join(QLatin1Char('\n')));
    m_hardwareHintLabel->setVisible(true);
}

ExtractionDialog::State ExtractionDialog::state() const
{
    return m_state;
}

ExtractionRequestSnapshot ExtractionDialog::buildRequest() const
{
    ExtractionRequestSnapshot request;
    if (m_selectedEntryRadio->isChecked() && m_selectedKey.has_value()) {
        request.entries = {*m_selectedKey};
    } else {
        request.entries = m_viewKeys;
    }
    request.hardware = m_profile;
    request.outputDir = m_outputDirEdit->text().trimmed();
    if (m_flatRadio->isChecked()) {
        request.pathMode = ExtractionPathMode::Flat;
    } else if (m_relativeToRadio->isChecked()) {
        request.pathMode = ExtractionPathMode::RelativeTo;
    } else {
        request.pathMode = ExtractionPathMode::Full;
    }
    request.relativeTo = m_relativeToEdit->text().trimmed();
    request.decode =
        m_decodeRadio->isChecked() ? ExtractionDecodeMode::Auto : ExtractionDecodeMode::Never;
    request.keepStored = m_keepStoredCheck->isChecked() && m_decodeRadio->isChecked();
    request.continueOnError = m_continueOnErrorCheck->isChecked();
    request.allowOverwrite = m_allowOverwriteCheck->isChecked();
    return request;
}

std::optional<ExtractionRequestSnapshot> ExtractionDialog::validatedRequest()
{
    const ExtractionRequestSnapshot request = buildRequest();
    // Local validation only; the Rust side stays the authority on
    // everything else (including the RelativeTo grammar).
    if (request.outputDir.isEmpty()) {
        m_statusLabel->setText(tr("An output directory is required."));
        return std::nullopt;
    }
    if (!QDir::isAbsolutePath(request.outputDir)) {
        m_statusLabel->setText(tr("The output directory must be an absolute path."));
        return std::nullopt;
    }
    if (request.pathMode == ExtractionPathMode::RelativeTo && request.relativeTo.isEmpty()) {
        m_statusLabel->setText(tr("A non-empty prefix is required for Relative to."));
        return std::nullopt;
    }
    return request;
}

void ExtractionDialog::onOptionsChanged()
{
    // Any change after (or during) a preflight invalidates the plan:
    // extraction always runs against a freshly checked request.
    if (m_state != State::Checking && m_state != State::PlanReady) {
        return;
    }
    emit requestInvalidated();
    m_state = State::Setup;
    m_statusLabel->clear();
    m_preflightButton->setEnabled(true);
    m_stack->setCurrentWidget(m_setupPage);
}

void ExtractionDialog::onBrowse()
{
    const QString path = QFileDialog::getExistingDirectory(this, tr("Output Directory"));
    if (!path.isEmpty()) {
        m_outputDirEdit->setText(path);
    }
}

void ExtractionDialog::onPreflight()
{
    const std::optional<ExtractionRequestSnapshot> request = validatedRequest();
    if (!request.has_value()) {
        return;
    }
    m_state = State::Checking;
    m_statusLabel->setText(tr("Checking..."));
    m_preflightButton->setEnabled(false);
    emit preflightRequested(*request);
}

void ExtractionDialog::onExtract()
{
    if (m_state != State::PlanReady) {
        return;
    }
    m_state = State::Running;
    m_stack->setCurrentWidget(m_runningPage);
    emit extractRequested(buildRequest());
}

void ExtractionDialog::onBackToSetup()
{
    // The plan stays valid until an option actually changes.
    m_stack->setCurrentWidget(m_setupPage);
}

void ExtractionDialog::onFailureBack()
{
    m_state = State::Setup;
    m_statusLabel->clear();
    m_preflightButton->setEnabled(true);
    m_stack->setCurrentWidget(m_setupPage);
}

void ExtractionDialog::showPlan(const ExtractionPlanSnapshot &plan)
{
    if (m_state != State::Checking) {
        // Anything the user has already moved on from: dropped.
        return;
    }
    m_state = State::PlanReady;
    m_planBodyLabel->setText(tr("Requested records:          %1\n"
                                "Omitted records:            %2\n"
                                "Hardware-excluded records:  %3\n"
                                "Planned records:            %4\n"
                                "Output paths:               %5\n"
                                "Existing outputs:           %6")
                                 .arg(plan.requestedRecords)
                                 .arg(plan.omittedRecords)
                                 .arg(plan.hardwareExcludedRecords)
                                 .arg(plan.plannedRecords)
                                 .arg(plan.outputPaths)
                                 .arg(plan.existingOutputs));
    // Overwriting is a warning, never an error.
    if (plan.existingOutputs > 0) {
        m_overwriteWarningLabel->setText(
            tr("⚠ %n existing regular files will be overwritten.", nullptr, plan.existingOutputs));
    } else {
        m_overwriteWarningLabel->clear();
    }
    m_stack->setCurrentWidget(m_planPage);
}

void ExtractionDialog::showPlanFailure(const QString &message)
{
    if (m_state != State::Checking) {
        return;
    }
    m_state = State::Failed;
    showFailurePage(tr("Extraction refused"), message, tr("No files were written."));
}

void ExtractionDialog::showReport(const ExtractionReportSnapshot &report)
{
    if (m_state != State::Running) {
        return;
    }
    m_state = State::Done;
    m_resultTitleLabel->setText(report.failures.isEmpty()
                                    ? tr("Extraction complete")
                                    : tr("Extraction completed with errors"));

    QString body;
    body += tr("Extracted:    %1\n").arg(report.extracted);
    body += tr("Skipped:      %1\n").arg(report.skipped);
    body += tr("Failed:       %1\n").arg(report.failures.size());
    body += tr("Recovered:    %1\n").arg(report.recoveries.size());

    if (!report.failures.isEmpty()) {
        body += tr("\nFailures:\n");
        for (const ExtractionFailureSnapshot &failure : report.failures) {
            body += QStringLiteral("  %1: %2\n").arg(failure.path, failure.message);
        }
    }
    if (!report.recoveries.isEmpty()) {
        body += tr("\nRecoveries:\n");
        for (const ExtractionRecoverySnapshot &recovery : report.recoveries) {
            body += QStringLiteral("  %1\n").arg(recovery.path);
            const QString expected = recovery.expectedKnown
                ? QStringLiteral("0x%1").arg(recovery.expectedOffset, 0, 16)
                : QStringLiteral("-");
            body += tr("    expected:   %1\n").arg(expected);
            body += QStringLiteral("    actual:     0x%1\n")
                        .arg(recovery.actualOffset, 0, 16);
            QString resolution;
            switch (recovery.kind) {
            case ExtractionRecoveryKind::Delta:
                resolution = tr("Delta");
                break;
            case ExtractionRecoveryKind::Resynced:
                resolution = tr("Resynced");
                break;
            case ExtractionRecoveryKind::Scanned:
                resolution = tr("Scanned");
                break;
            }
            if (recovery.deltaKnown && recovery.delta >= 0) {
                resolution += QStringLiteral(" +%1").arg(recovery.delta);
            } else if (recovery.deltaKnown) {
                resolution += QStringLiteral(" %1").arg(recovery.delta);
            }
            body += tr("    resolution: %1\n").arg(resolution);
        }
    }
    m_resultBodyEdit->setPlainText(body);
    m_stack->setCurrentWidget(m_resultPage);
}

void ExtractionDialog::showExtractionFailure(const QString &message)
{
    if (m_state != State::Running) {
        return;
    }
    m_state = State::Failed;
    // A refusal at execution time: the re-evaluated plan rejected the
    // extraction before anything was written.
    showFailurePage(tr("Extraction refused"),
                    message,
                    tr("No files were written by this extraction attempt."));
}

void ExtractionDialog::showFailurePage(const QString &title,
                                       const QString &message,
                                       const QString &note)
{
    m_failureTitleLabel->setText(title);
    m_failureMessageEdit->setPlainText(message);
    m_failureNoteLabel->setText(note);
    m_preflightButton->setEnabled(true);
    m_stack->setCurrentWidget(m_failurePage);
}

void ExtractionDialog::reject()
{
    if (m_state == State::Running) {
        // No cancellation and no fake one: the batch runs to
        // completion; close, reject and Esc are all blocked.
        return;
    }
    if (m_state == State::Checking) {
        emit requestInvalidated();
    }
    QDialog::reject();
}
