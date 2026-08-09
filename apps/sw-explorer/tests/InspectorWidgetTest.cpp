#include "InspectorWidget.h"

#include <QFormLayout>
#include <QGroupBox>
#include <QLabel>
#include <QStackedWidget>
#include <QTest>

// Drives InspectorWidget with hand-built snapshots and checks both
// the page stack and the section content: which sections exist, what
// text they carry, and that unknown is never rendered as empty.
class InspectorWidgetTest : public QObject
{
    Q_OBJECT

private slots:
    void emptyPageIsInitial();
    void loadingPage();
    void errorPage();
    void productRendering();
    void productWithoutDescriptorHidesDescriptorData();
    void imageRendering();
    void subsystemRendering();
    void idbOnlySubsystemShowsUnknownNotEmpty();
};

namespace {

// All QLabel texts anywhere below a widget, for content assertions.
QStringList allLabelTexts(QWidget *root)
{
    QStringList texts;
    for (QLabel *label : root->findChildren<QLabel *>()) {
        texts.append(label->text());
    }
    return texts;
}

QStackedWidget *stackOf(InspectorWidget &inspector)
{
    return inspector.findChild<QStackedWidget *>();
}

QString currentPageName(InspectorWidget &inspector)
{
    return stackOf(inspector)->currentWidget()->objectName();
}

// The content page carries its kind-specific objectName on the scroll
// area inside the stacked content slot.
QString contentPageName(InspectorWidget &inspector)
{
    QWidget *content = stackOf(inspector)->currentWidget();
    const QList<QWidget *> children = content->findChildren<QWidget *>();
    for (QWidget *child : children) {
        if (child->objectName().endsWith(QStringLiteral("Page"))) {
            return child->objectName();
        }
    }
    return QString();
}

RangeSnapshot range(const QString &target, quint64 min, quint64 max, bool unbounded = false)
{
    RangeSnapshot out;
    out.target = target;
    out.minVersion = min;
    out.maxVersion = max;
    out.maxIsUnbounded = unbounded;
    return out;
}

} // namespace

void InspectorWidgetTest::emptyPageIsInitial()
{
    InspectorWidget inspector;
    QCOMPARE(currentPageName(inspector), QStringLiteral("emptyPage"));

    inspector.showEmpty();
    QCOMPARE(currentPageName(inspector), QStringLiteral("emptyPage"));
}

void InspectorWidgetTest::loadingPage()
{
    InspectorWidget inspector;
    inspector.showLoading();
    QCOMPARE(currentPageName(inspector), QStringLiteral("loadingPage"));
}

void InspectorWidgetTest::errorPage()
{
    InspectorWidget inspector;
    inspector.showError(QStringLiteral("object 123 no longer exists"));
    QCOMPARE(currentPageName(inspector), QStringLiteral("errorPage"));

    auto *message = inspector.findChild<QLabel *>(QStringLiteral("errorMessage"));
    QVERIFY(message != nullptr);
    QCOMPARE(message->text(), QStringLiteral("object 123 no longer exists"));
    // Media data is shown verbatim, never reinterpreted as rich text.
    QCOMPARE(message->textFormat(), Qt::PlainText);
}

void InspectorWidgetTest::productRendering()
{
    ProductDetailSnapshot detail;
    detail.name = QStringLiteral("eoe");
    detail.title = QStringLiteral("IRIX Execution Environment");
    detail.descriptorPresent = true;
    detail.idbPresent = true;
    detail.imageCount = 3;
    detail.subsystemCount = 27;
    detail.entryCount = 18452;
    detail.descriptorGeneration = QStringLiteral("pd001V630P00");
    detail.descriptorLayoutLevel = 9;
    detail.descriptorStamp = 424242;
    detail.mach.known = true;
    detail.mach.expressions = {QStringLiteral("CPUBOARD=IP22")};
    detail.cutpoints = {{QStringLiteral("/usr/share/catman"), 7}};

    InspectorWidget inspector;
    inspector.showProduct(detail);

    QCOMPARE(contentPageName(inspector), QStringLiteral("productPage"));
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("generalSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("descriptorSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("hardwareSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("cutpointsSection")) != nullptr);

    const QStringList texts = allLabelTexts(&inspector);
    QVERIFY(texts.contains(QStringLiteral("eoe")));
    QVERIFY(texts.contains(QStringLiteral("IRIX Execution Environment")));
    QVERIFY(texts.contains(QStringLiteral("3")));
    QVERIFY(texts.contains(QStringLiteral("27")));
    QVERIFY(texts.contains(QStringLiteral("18452")));
    QVERIFY(texts.contains(QStringLiteral("pd001V630P00")));
    QVERIFY(texts.contains(QStringLiteral("9")));
    QVERIFY(texts.contains(QStringLiteral("424242")));
    QVERIFY(texts.contains(QStringLiteral("CPUBOARD=IP22")));
    QVERIFY(texts.contains(QStringLiteral("/usr/share/catman")));
    QVERIFY(texts.contains(QStringLiteral("7")));
}

void InspectorWidgetTest::productWithoutDescriptorHidesDescriptorData()
{
    ProductDetailSnapshot detail;
    detail.name = QStringLiteral("test");
    // No title, no descriptor, no MACH, no cutpoints.
    detail.idbPresent = true;
    detail.imageCount = 2;
    detail.subsystemCount = 2;
    detail.entryCount = 5;

    InspectorWidget inspector;
    inspector.showProduct(detail);

    QCOMPARE(contentPageName(inspector), QStringLiteral("productPage"));
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("generalSection")) != nullptr);
    // Unknown descriptor data is hidden, not rendered as empty.
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("descriptorSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("hardwareSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("cutpointsSection")) == nullptr);
    // No title: no subtitle label.
    QVERIFY(inspector.findChild<QLabel *>(QStringLiteral("titleLabel")) == nullptr);

    const QStringList texts = allLabelTexts(&inspector);
    QVERIFY(!texts.contains(QStringLiteral("pd001V630P00")));
}

void InspectorWidgetTest::imageRendering()
{
    ImageDetailSnapshot detail;
    detail.name = QStringLiteral("eoe.sw");
    detail.title = QStringLiteral("System Software");
    detail.productName = QStringLiteral("eoe");
    detail.descriptorPresent = true;
    detail.idbPresent = true;
    detail.versionKnown = true;
    detail.version = 1021572036;
    detail.orderKnown = true;
    detail.order = 10;
    detail.subsystemCount = 27;
    detail.entryCount = 18452;
    detail.mach.known = true; // known, but unrestricted

    InspectorWidget inspector;
    inspector.showImage(detail);

    QCOMPARE(contentPageName(inspector), QStringLiteral("imagePage"));
    // Known-unrestricted hardware hides the section.
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("hardwareSection")) == nullptr);

    QStringList texts = allLabelTexts(&inspector);
    QVERIFY(texts.contains(QStringLiteral("eoe.sw")));
    QVERIFY(texts.contains(QStringLiteral("eoe")));
    QVERIFY(texts.contains(QStringLiteral("1021572036")));
    QVERIFY(texts.contains(QStringLiteral("10")));

    // Unknown version/order render as "-", never as a fake 0.
    detail.versionKnown = false;
    detail.orderKnown = false;
    inspector.showImage(detail);
    texts = allLabelTexts(&inspector);
    QVERIFY(texts.contains(QStringLiteral("-")));
    QVERIFY(!texts.contains(QStringLiteral("1021572036")));
}

void InspectorWidgetTest::subsystemRendering()
{
    SubsystemDetailSnapshot detail;
    detail.identity = QStringLiteral("eoe.sw.gfx");
    detail.shortName = QStringLiteral("gfx");
    detail.title = QStringLiteral("Graphics Execution Environment");
    detail.productName = QStringLiteral("eoe");
    detail.imageName = QStringLiteral("eoe.sw");
    detail.imageVersionKnown = true;
    detail.imageVersion = 1021572036;
    detail.descriptorPresent = true;
    detail.idbPresent = true;
    detail.entryCount = 115;
    detail.mappingKnown = true;
    detail.mapping = QStringLiteral("ALL");

    detail.mach.known = true;
    detail.mach.expressions = {QStringLiteral("GFXBOARD=NEWPRESS"),
                               QStringLiteral("GFXBOARD=EXPRESS")};
    detail.mach.unresolved = {QStringLiteral("=GARBAGE")};

    detail.flags.known = true;
    detail.flags.required.state = ConditionalFlagState::Unresolved;
    detail.flags.required.unresolved = {QStringLiteral("=BROKEN")};
    detail.flags.defaultFlag.state = ConditionalFlagState::Conditional;
    detail.flags.defaultFlag.expressions = {QStringLiteral("CPUBOARD=IP22")};
    detail.flags.miniroot.state = ConditionalFlagState::Always;
    detail.flags.patch = true;

    detail.rules.known = true;
    PrerequisiteClauseSnapshot clause;
    clause.allOf = {range(QStringLiteral("eoe.sw.gfx"), 1, 10),
                    range(QStringLiteral("eoe.sw.unix"), 2, 20)};
    detail.rules.prerequisites = {
        clause,
        {QList<RangeSnapshot>{range(QStringLiteral("foo.sw.bar"), 0, 0, true)}}};
    detail.rules.replaces = {range(QStringLiteral("patch*.sw.unix"), 0, 1274627332)};
    detail.rules.updates = {range(QStringLiteral("old.sw.thing"), 3, 9)};
    detail.rules.follows = {range(QStringLiteral("eoe.sw.base"), 5, 100)};
    detail.autominirootKnown = true;
    detail.autominiroot = {range(QStringLiteral("eoe.sw.unix"), 0, 1274627332)};

    InspectorWidget inspector;
    inspector.showSubsystem(detail);

    QCOMPARE(contentPageName(inspector), QStringLiteral("subsystemPage"));
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("flagsSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("hardwareSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("unresolvedMachSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("mappingSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("prerequisitesSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("replacesSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("updatesSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("followsSection")) != nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("autominirootSection")) != nullptr);

    const QStringList texts = allLabelTexts(&inspector);
    QVERIFY(texts.contains(QStringLiteral("eoe.sw.gfx")));
    QVERIFY(texts.contains(QStringLiteral("Graphics Execution Environment")));
    // The labels we create for media data render as plain text,
    // exactly as stored (QFormLayout's internal row-name labels are
    // not ours to configure).
    QCOMPARE(inspector.findChild<QLabel *>(QStringLiteral("nameLabel"))->textFormat(),
             Qt::PlainText);
    QCOMPARE(inspector.findChild<QLabel *>(QStringLiteral("titleLabel"))->textFormat(),
             Qt::PlainText);
    QVERIFY(texts.contains(QStringLiteral("GFXBOARD=NEWPRESS")));
    QVERIFY(texts.contains(QStringLiteral("=GARBAGE")));
    QVERIFY(texts.contains(QStringLiteral("when CPUBOARD=IP22")));
    QVERIFY(texts.filter(QStringLiteral("unresolved condition: =BROKEN")).size() == 1);
    QVERIFY(texts.contains(QStringLiteral("Miniroot")));
    QVERIFY(texts.contains(QStringLiteral("Patch")));
    QVERIFY(texts.contains(QStringLiteral("ALL")));
    // The AND within a clause stays on one line, the OR clauses stay
    // on separate lines.
    QVERIFY(texts.contains(QStringLiteral("eoe.sw.gfx 1..10 + eoe.sw.unix 2..20")));
    QVERIFY(texts.contains(QStringLiteral("foo.sw.bar 0..maxint")));
    QVERIFY(texts.contains(QStringLiteral("patch*.sw.unix 0..1274627332")));
    QVERIFY(texts.contains(QStringLiteral("old.sw.thing 3..9")));
    QVERIFY(texts.contains(QStringLiteral("eoe.sw.base 5..100")));
    QVERIFY(texts.contains(QStringLiteral("eoe.sw.unix 0..1274627332")));
    // "No" flags are never listed.
    QVERIFY(!texts.contains(QStringLiteral("In-place")));
    QVERIFY(!texts.contains(QStringLiteral("Client-only")));
}

void InspectorWidgetTest::idbOnlySubsystemShowsUnknownNotEmpty()
{
    SubsystemDetailSnapshot detail;
    detail.identity = QStringLiteral("mixed.sw.extra");
    detail.shortName = QStringLiteral("extra");
    detail.productName = QStringLiteral("mixed");
    detail.imageName = QStringLiteral("mixed.sw");
    // No descriptor record: everything descriptor-derived is unknown.
    detail.descriptorPresent = false;
    detail.idbPresent = true;
    detail.entryCount = 3;

    InspectorWidget inspector;
    inspector.showSubsystem(detail);

    QCOMPARE(contentPageName(inspector), QStringLiteral("subsystemPage"));
    auto *note = inspector.findChild<QLabel *>(QStringLiteral("descriptorUnavailableNote"));
    QVERIFY(note != nullptr);
    QCOMPARE(note->text(), QStringLiteral("Descriptor information is unavailable."));

    // Unknown sections are hidden, not rendered as "none".
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("flagsSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("hardwareSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("mappingSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("prerequisitesSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("replacesSection")) == nullptr);
    QVERIFY(inspector.findChild<QWidget *>(QStringLiteral("autominirootSection")) == nullptr);

    const QStringList texts = allLabelTexts(&inspector);
    QVERIFY(texts.contains(QStringLiteral("No"))); // Descriptor: No
    QVERIFY(texts.contains(QStringLiteral("Yes"))); // IDB: Yes
    QVERIFY(texts.contains(QStringLiteral("3")));
}

QTEST_MAIN(InspectorWidgetTest)

#include "InspectorWidgetTest.moc"
