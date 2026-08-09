#include "HardwareProfileDialog.h"

#include <QComboBox>
#include <QLineEdit>
#include <QMessageBox>
#include <QPushButton>
#include <QSignalSpy>
#include <QTableView>
#include <QTest>
#include <QTimer>

#include "models/HardwareProfileModel.h"

// Drives the hardware profile dialog with its delegate: editable
// combobox editors, canonical and distribution-discovered
// suggestions, hand-typed values, and the Add / Remove / Clear /
// Cancel / Apply protocol.
class HardwareProfileDialogTest : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void editorsAreEditableComboBoxes();
    void canonicalAttributesAreAlwaysOffered();
    void discoveredUnknownAttributeIsOffered();
    void valueSuggestionsFollowTheRowAttribute();
    void handTypedValueOutsideSuggestionsIsAccepted();
    void currentValueSurvivesWithoutSuggestion();
    void emptyValuePlaceholderAppliesAsEmptyString();
    void multiValuedAttributeRowsCoexist();
    void clearEmptiesTheEditor();
    void cancelLeavesTheProfileUntouched();
    void blankRowsAreDroppedOnApply();
    void halfFilledRowBlocksApply();
    void applyEmitsTrimmedProfile();
    void applyCommitsTheStillOpenEditor();
};

namespace {

QTableView *tableOf(HardwareProfileDialog &dialog)
{
    return dialog.findChild<QTableView *>(QStringLiteral("profileTable"));
}

QPushButton *buttonOf(HardwareProfileDialog &dialog, const QString &objectName)
{
    return dialog.findChild<QPushButton *>(objectName);
}

// Opens the combobox editor of one cell; the dialog must be shown.
QComboBox *openEditor(QTableView *view, const QModelIndex &index)
{
    view->setCurrentIndex(index);
    view->edit(index);
    return view->findChild<QComboBox *>();
}

// Moves focus off the open editor and lets the deferred FocusOut
// commit land: the delegate's FocusOut handling commits and closes
// the editor, but asynchronously. (Key-based commits are unreliable
// for combobox editors: the delegate deliberately lets QComboBox eat
// Return for its popup.)
void closeEditor(QTableView *view)
{
    view->setFocus(Qt::OtherFocusReason);
    QCoreApplication::processEvents();
}

// Types text into the open editor (replacing the current text) and
// commits it.
void typeAndCommit(QTableView *view, QComboBox *combo, const QString &text)
{
    combo->lineEdit()->selectAll();
    QTest::keyClicks(combo->lineEdit(), text);
    closeEditor(view);
}

HardwareCandidatesSnapshot sampleCandidates()
{
    return {{QStringLiteral("CPUBOARD"), {QStringLiteral("IP22"), QStringLiteral("IP26")}},
            {QStringLiteral("GFXBOARD"), {QStringLiteral("EXPRESS")}},
            {QStringLiteral("FROBNICATE"), {QStringLiteral("YES")}}};
}

} // namespace

void HardwareProfileDialogTest::initTestCase()
{
    qRegisterMetaType<HardwareProfileSnapshot>("HardwareProfileSnapshot");
}

void HardwareProfileDialogTest::editorsAreEditableComboBoxes()
{
    HardwareProfileDialog dialog;
    dialog.setCandidates(sampleCandidates());
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *attributeEditor = openEditor(view, view->model()->index(0, 0));
    QVERIFY(attributeEditor != nullptr);
    QVERIFY(attributeEditor->isEditable());
    closeEditor(view);

    QComboBox *valueEditor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(valueEditor != nullptr);
    QVERIFY(valueEditor->isEditable());
    closeEditor(view);
}

void HardwareProfileDialogTest::canonicalAttributesAreAlwaysOffered()
{
    // No distribution candidates at all: the grammar's attribute
    // names are still offered.
    HardwareProfileDialog dialog;
    dialog.setProfile({{QString(), QString()}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 0));
    QVERIFY(editor != nullptr);
    for (const QString &attribute :
         {QStringLiteral("CPUBOARD"), QStringLiteral("CPUARCH"), QStringLiteral("GFXBOARD"),
          QStringLiteral("SUBGR"), QStringLiteral("VIDEO"), QStringLiteral("MODE"),
          QStringLiteral("TARGOS"), QStringLiteral("DISTOS")}) {
        QVERIFY2(editor->findText(attribute) >= 0, qPrintable(attribute));
    }
    // The "(empty)" placeholder belongs to the Value column only; an
    // attribute name is never empty.
    QVERIFY(editor->findText(QStringLiteral("(empty)")) < 0);
    closeEditor(view);
}

void HardwareProfileDialogTest::discoveredUnknownAttributeIsOffered()
{
    HardwareProfileDialog dialog;
    dialog.setCandidates(sampleCandidates());
    dialog.setProfile({{QString(), QString()}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 0));
    QVERIFY(editor != nullptr);
    // The distribution-discovered unknown attribute joins the
    // canonical names.
    QVERIFY(editor->findText(QStringLiteral("FROBNICATE")) >= 0);
    QVERIFY(editor->findText(QStringLiteral("CPUBOARD")) >= 0);
    closeEditor(view);
}

void HardwareProfileDialogTest::valueSuggestionsFollowTheRowAttribute()
{
    HardwareProfileDialog dialog;
    dialog.setCandidates(sampleCandidates());
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QString()}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);
    QVERIFY(editor->findText(QStringLiteral("IP22")) >= 0);
    QVERIFY(editor->findText(QStringLiteral("IP26")) >= 0);
    // Values of other attributes are not suggested for this row.
    QVERIFY(editor->findText(QStringLiteral("EXPRESS")) < 0);
    closeEditor(view);
}

void HardwareProfileDialogTest::handTypedValueOutsideSuggestionsIsAccepted()
{
    HardwareProfileDialog dialog;
    dialog.setCandidates(sampleCandidates());
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QString()}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    QTableView *view = tableOf(dialog);

    // A value the suggestions do not know can simply be typed.
    QComboBox *editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);
    typeAndCommit(view, editor, QStringLiteral("IP99"));

    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 1);
    QCOMPARE(applied.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(applied.at(0).value, QStringLiteral("IP99"));
}

void HardwareProfileDialogTest::currentValueSurvivesWithoutSuggestion()
{
    HardwareProfileDialog dialog;
    // The candidates come from a different distribution that never
    // mentions IP99; the profile's own value must still be offered
    // and survive the roundtrip.
    dialog.setCandidates(sampleCandidates());
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP99")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);
    QCOMPARE(editor->currentText(), QStringLiteral("IP99"));
    QVERIFY(editor->findText(QStringLiteral("IP99")) >= 0);
    closeEditor(view);

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 1);
    QCOMPARE(applied.at(0).value, QStringLiteral("IP99"));
}

void HardwareProfileDialogTest::emptyValuePlaceholderAppliesAsEmptyString()
{
    HardwareProfileDialog dialog;
    // The distribution carries `GFXBOARD=` (headless boards): the
    // empty value is a real candidate.
    dialog.setCandidates(
        {{QStringLiteral("GFXBOARD"),
          {QStringLiteral("EXPRESS"), QString()}}});
    dialog.setProfile({{QStringLiteral("GFXBOARD"), QStringLiteral("EXPRESS")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);

    // The empty candidate is offered as a visible placeholder, never
    // as a blank entry that looks like "nothing chosen".
    const int placeholder = editor->findText(QStringLiteral("(empty)"));
    QVERIFY(placeholder >= 0);
    editor->setCurrentIndex(placeholder);
    QCOMPARE(editor->currentText(), QStringLiteral("(empty)"));
    closeEditor(view);

    // The placeholder commits the real value: the empty string.
    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 1);
    QCOMPARE(applied.at(0).attribute, QStringLiteral("GFXBOARD"));
    QCOMPARE(applied.at(0).value, QString());
}

void HardwareProfileDialogTest::multiValuedAttributeRowsCoexist()
{
    HardwareProfileDialog dialog;
    dialog.setProfile({{QStringLiteral("CPUARCH"), QStringLiteral("MIPS2")},
                       {QStringLiteral("CPUARCH"), QStringLiteral("R4000")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 2);
    QCOMPARE(applied.at(0).value, QStringLiteral("MIPS2"));
    QCOMPARE(applied.at(1).value, QStringLiteral("R4000"));
}

void HardwareProfileDialogTest::clearEmptiesTheEditor()
{
    HardwareProfileDialog dialog;
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QCOMPARE(view->model()->rowCount(), 1);
    buttonOf(dialog, QStringLiteral("clearButton"))->click();
    QCOMPARE(view->model()->rowCount(), 0);

    // Applying the empty profile disables the hardware selection.
    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    QVERIFY(qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0)).isEmpty());
}

void HardwareProfileDialogTest::cancelLeavesTheProfileUntouched()
{
    HardwareProfileDialog dialog;
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    // Edit a cell, then cancel: nothing is applied.
    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);
    typeAndCommit(view, editor, QStringLiteral("IP26"));

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("cancelButton"))->click();
    QCOMPARE(spy.count(), 0);
    QVERIFY(!dialog.isVisible());
}

void HardwareProfileDialogTest::blankRowsAreDroppedOnApply()
{
    HardwareProfileDialog dialog;
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    // Add two rows; leave one completely blank and fill the other.
    buttonOf(dialog, QStringLiteral("addButton"))->click();
    closeEditor(tableOf(dialog));
    buttonOf(dialog, QStringLiteral("addButton"))->click();
    closeEditor(tableOf(dialog));

    QTableView *view = tableOf(dialog);
    QCOMPARE(view->model()->rowCount(), 3);
    QComboBox *editor = openEditor(view, view->model()->index(2, 0));
    QVERIFY(editor != nullptr);
    typeAndCommit(view, editor, QStringLiteral("MODE"));
    editor = openEditor(view, view->model()->index(2, 1));
    QVERIFY(editor != nullptr);
    typeAndCommit(view, editor, QStringLiteral("32bit"));

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 2);
    QCOMPARE(applied.at(0).value, QStringLiteral("IP22"));
    QCOMPARE(applied.at(1).attribute, QStringLiteral("MODE"));
    QCOMPARE(applied.at(1).value, QStringLiteral("32bit"));
}

void HardwareProfileDialogTest::halfFilledRowBlocksApply()
{
    // A value without an attribute is a mistake; an attribute with an
    // empty value is not (the media carry `GFXBOARD=`).
    HardwareProfileDialog dialog;
    dialog.setProfile({{QString(), QStringLiteral("IP22")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    // The validation message box is modal; accept it from a watcher.
    bool warningSeen = false;
    QTimer closer;
    closer.setInterval(50);
    connect(&closer, &QTimer::timeout, [&warningSeen] {
        const QWidgetList topLevels = QApplication::topLevelWidgets();
        for (QWidget *topLevel : topLevels) {
            if (auto *box = qobject_cast<QMessageBox *>(topLevel)) {
                warningSeen = true;
                box->accept();
            }
        }
    });
    closer.start();

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QTRY_VERIFY(warningSeen);
    closer.stop();

    // No profile left the dialog; the dialog stays open.
    QCOMPARE(spy.count(), 0);
    QVERIFY(dialog.isVisible());
}

void HardwareProfileDialogTest::applyEmitsTrimmedProfile()
{
    HardwareProfileDialog dialog;
    dialog.setProfile({{QString(), QString()}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 0));
    QVERIFY(editor != nullptr);
    typeAndCommit(view, editor, QStringLiteral("  CPUBOARD "));
    editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);
    typeAndCommit(view, editor, QStringLiteral(" IP22\t"));

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 1);
    QCOMPARE(applied.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(applied.at(0).value, QStringLiteral("IP22"));
}

void HardwareProfileDialogTest::applyCommitsTheStillOpenEditor()
{
    HardwareProfileDialog dialog;
    dialog.setCandidates(sampleCandidates());
    dialog.setProfile({{QStringLiteral("CPUBOARD"), QStringLiteral("IP22")}});
    dialog.show();
    QVERIFY(QTest::qWaitForWindowExposed(&dialog));

    // Type into the value editor and go straight to Apply: no
    // FocusOut, no closeEditor. FocusOut commits are deferred, so the
    // dialog must pull the editor's text in itself.
    QTableView *view = tableOf(dialog);
    QComboBox *editor = openEditor(view, view->model()->index(0, 1));
    QVERIFY(editor != nullptr);
    editor->lineEdit()->selectAll();
    QTest::keyClicks(editor->lineEdit(), QStringLiteral("IP26"));

    QSignalSpy spy(&dialog, &HardwareProfileDialog::profileApplied);
    buttonOf(dialog, QStringLiteral("applyButton"))->click();
    QCOMPARE(spy.count(), 1);
    const HardwareProfileSnapshot applied =
        qvariant_cast<HardwareProfileSnapshot>(spy.first().at(0));
    QCOMPARE(applied.size(), 1);
    QCOMPARE(applied.at(0).attribute, QStringLiteral("CPUBOARD"));
    QCOMPARE(applied.at(0).value, QStringLiteral("IP26"));
}

QTEST_MAIN(HardwareProfileDialogTest)

#include "HardwareProfileDialogTest.moc"
