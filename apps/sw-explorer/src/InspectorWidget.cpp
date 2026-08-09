#include "InspectorWidget.h"

#include <QApplication>
#include <QFormLayout>
#include <QGroupBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QScrollArea>
#include <QStackedWidget>
#include <QStyle>
#include <QVBoxLayout>

namespace {

// A detail value label; selectable so paths and expressions can be
// copied out. PlainText: data from the media is shown verbatim, never
// reinterpreted as rich text.
QLabel *textLabel(const QString &text)
{
    auto *label = new QLabel(text);
    label->setTextFormat(Qt::PlainText);
    label->setTextInteractionFlags(Qt::TextSelectableByMouse);
    label->setWordWrap(true);
    return label;
}

// A value row with a warning icon, for data that could not be fully
// decoded (unresolved MACH, unresolved flag conditions). Not an
// error, but never silently dropped either.
QWidget *warningRow(const QString &text)
{
    auto *row = new QWidget;
    auto *layout = new QHBoxLayout(row);
    layout->setContentsMargins(0, 0, 0, 0);
    auto *icon = new QLabel;
    const int size = QApplication::style()->pixelMetric(QStyle::PM_SmallIconSize);
    icon->setPixmap(
        QApplication::style()->standardIcon(QStyle::SP_MessageBoxWarning).pixmap(size, size));
    icon->setFixedSize(size, size);
    layout->addWidget(icon, 0, Qt::AlignTop);
    layout->addWidget(textLabel(text), 1);
    return row;
}

QVBoxLayout *startSection(QVBoxLayout *parent, const QString &title, const QString &objectName)
{
    auto *box = new QGroupBox(title);
    box->setObjectName(objectName);
    auto *layout = new QVBoxLayout(box);
    parent->addWidget(box);
    return layout;
}

QFormLayout *startFormSection(QVBoxLayout *parent, const QString &title, const QString &objectName)
{
    auto *box = new QGroupBox(title);
    box->setObjectName(objectName);
    auto *layout = new QFormLayout(box);
    parent->addWidget(box);
    return layout;
}

void addFormRow(QFormLayout *form, const QString &name, const QString &value)
{
    form->addRow(name, textLabel(value));
}

QString yesNo(bool value)
{
    return value ? InspectorWidget::tr("Yes") : InspectorWidget::tr("No");
}

// The CLI range spelling, e.g. "patch*.sw.unix 0..1274627332" or
// "foo.sw.bar 0..maxint".
QString rangeText(const RangeSnapshot &range)
{
    const QString max =
        range.maxIsUnbounded ? QStringLiteral("maxint") : QString::number(range.maxVersion);
    return QStringLiteral("%1 %2..%3")
        .arg(range.target, QString::number(range.minVersion), max);
}

// The Hardware section pair: parsed expressions, and — clearly
// separated — the raw payloads that could not be parsed. Hidden when
// unknown or known-unrestricted.
void addHardwareSections(QVBoxLayout *parent, const HardwareSnapshot &mach)
{
    if (!mach.known) {
        return;
    }
    if (!mach.expressions.isEmpty()) {
        QVBoxLayout *section =
            startSection(parent, InspectorWidget::tr("Hardware"), QStringLiteral("hardwareSection"));
        for (const QString &expression : mach.expressions) {
            section->addWidget(textLabel(expression));
        }
    }
    if (!mach.unresolved.isEmpty()) {
        QVBoxLayout *section = startSection(parent,
                                            InspectorWidget::tr("Unresolved MACH"),
                                            QStringLiteral("unresolvedMachSection"));
        for (const QString &raw : mach.unresolved) {
            section->addWidget(warningRow(raw));
        }
    }
}

// One section per non-empty rule list; each range on its own line.
void addRangeSection(QVBoxLayout *parent,
                     const QString &title,
                     const QString &objectName,
                     const QList<RangeSnapshot> &ranges)
{
    if (ranges.isEmpty()) {
        return;
    }
    QVBoxLayout *section = startSection(parent, title, objectName);
    for (const RangeSnapshot &range : ranges) {
        section->addWidget(textLabel(rangeText(range)));
    }
}

// Prerequisites keep their two-level structure: one line per clause
// (clauses are OR-ed), ranges within a clause joined by " + " (AND).
void addPrerequisitesSection(QVBoxLayout *parent, const RulesSnapshot &rules)
{
    if (rules.prerequisites.isEmpty()) {
        return;
    }
    QVBoxLayout *section = startSection(parent,
                                        InspectorWidget::tr("Prerequisites"),
                                        QStringLiteral("prerequisitesSection"));
    for (const PrerequisiteClauseSnapshot &clause : rules.prerequisites) {
        QStringList ranges;
        ranges.reserve(clause.allOf.size());
        for (const RangeSnapshot &range : clause.allOf) {
            ranges.append(rangeText(range));
        }
        section->addWidget(textLabel(ranges.join(QStringLiteral(" + "))));
    }
}

// One flag row: plain flags span the row, conditional flags show
// their condition, unresolved conditions get a warning marker.
void addFlagRow(QFormLayout *form, const QString &name, const ConditionalFlagSnapshot &flag)
{
    switch (flag.state) {
    case ConditionalFlagState::No:
        break;
    case ConditionalFlagState::Always:
        form->addRow(textLabel(name));
        break;
    case ConditionalFlagState::Conditional:
        form->addRow(
            name,
            textLabel(InspectorWidget::tr("when %1")
                          .arg(flag.expressions.join(QStringLiteral(" or ")))));
        break;
    case ConditionalFlagState::Unresolved: {
        const QString condition = flag.unresolved.join(QStringLiteral(", "));
        const QString text =
            flag.expressions.isEmpty()
                ? InspectorWidget::tr("unresolved condition: %1").arg(condition)
                : InspectorWidget::tr("when %1 (unresolved condition: %2)")
                      .arg(flag.expressions.join(QStringLiteral(" or ")), condition);
        form->addRow(name, warningRow(text));
        break;
    }
    }
}

bool anyFlagSet(const SubsystemFlagsSnapshot &flags)
{
    return flags.required.state != ConditionalFlagState::No
        || flags.defaultFlag.state != ConditionalFlagState::No
        || flags.miniroot.state != ConditionalFlagState::No || flags.inplace || flags.patch
        || flags.clientOnly || flags.overlay || flags.overlayMember;
}

// The Flags section lists only the flags that are actually set; it is
// hidden entirely when the flags are unknown (IDB-only) or when the
// descriptor confirms none are set.
void addFlagsSection(QVBoxLayout *parent, const SubsystemFlagsSnapshot &flags)
{
    if (!flags.known || !anyFlagSet(flags)) {
        return;
    }
    QFormLayout *form =
        startFormSection(parent, InspectorWidget::tr("Flags"), QStringLiteral("flagsSection"));
    addFlagRow(form, InspectorWidget::tr("Required"), flags.required);
    addFlagRow(form, InspectorWidget::tr("Default"), flags.defaultFlag);
    addFlagRow(form, InspectorWidget::tr("Miniroot"), flags.miniroot);
    if (flags.inplace) {
        form->addRow(textLabel(InspectorWidget::tr("In-place")));
    }
    if (flags.patch) {
        form->addRow(textLabel(InspectorWidget::tr("Patch")));
    }
    if (flags.clientOnly) {
        form->addRow(textLabel(InspectorWidget::tr("Client-only")));
    }
    if (flags.overlay) {
        form->addRow(textLabel(InspectorWidget::tr("Overlay")));
    }
    if (flags.overlayMember) {
        form->addRow(textLabel(InspectorWidget::tr("Overlay member")));
    }
}

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

InspectorWidget::InspectorWidget(QWidget *parent)
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
    auto *errorTitle = new QLabel(tr("Unable to load details"), m_errorPage);
    errorTitle->setObjectName(QStringLiteral("errorTitle"));
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

    m_contentPage = new QWidget;
    new QVBoxLayout(m_contentPage);

    m_stack->addWidget(m_emptyPage);
    m_stack->addWidget(m_loadingPage);
    m_stack->addWidget(m_errorPage);
    m_stack->addWidget(m_contentPage);
    m_stack->setCurrentWidget(m_emptyPage);
}

void InspectorWidget::showEmpty()
{
    m_stack->setCurrentWidget(m_emptyPage);
}

void InspectorWidget::showLoading()
{
    m_stack->setCurrentWidget(m_loadingPage);
}

void InspectorWidget::showError(const QString &message)
{
    m_errorMessage->setText(message);
    m_stack->setCurrentWidget(m_errorPage);
}

// Replaces the content page with a scrollable rendering of the given
// detail page. Sections that do not apply were never added by the
// builder, so a hidden section is simply absent from the widget tree.
void InspectorWidget::showContent(const QString &pageName, QWidget *content)
{
    auto *pageLayout = qobject_cast<QVBoxLayout *>(m_contentPage->layout());
    while (QLayoutItem *item = pageLayout->takeAt(0)) {
        delete item->widget();
        delete item;
    }

    auto *scrollArea = new QScrollArea(m_contentPage);
    scrollArea->setObjectName(pageName);
    scrollArea->setWidgetResizable(true);
    scrollArea->setFrameShape(QFrame::NoFrame);
    scrollArea->setWidget(content);
    pageLayout->addWidget(scrollArea);
    m_stack->setCurrentWidget(m_contentPage);
}

void InspectorWidget::showProduct(const ProductDetailSnapshot &detail)
{
    showContent(QStringLiteral("productPage"), buildProductPage(detail));
}

void InspectorWidget::showImage(const ImageDetailSnapshot &detail)
{
    showContent(QStringLiteral("imagePage"), buildImagePage(detail));
}

void InspectorWidget::showSubsystem(const SubsystemDetailSnapshot &detail)
{
    showContent(QStringLiteral("subsystemPage"), buildSubsystemPage(detail));
}

namespace {

// The shared content scaffold: bold name header, optional subtitle,
// then the sections.
QWidget *makeContent(const QString &name, const QString &title, QVBoxLayout **outLayout)
{
    auto *page = new QWidget;
    auto *layout = new QVBoxLayout(page);

    auto *nameLabel = new QLabel(name, page);
    nameLabel->setObjectName(QStringLiteral("nameLabel"));
    nameLabel->setTextFormat(Qt::PlainText);
    nameLabel->setTextInteractionFlags(Qt::TextSelectableByMouse);
    QFont nameFont = nameLabel->font();
    nameFont.setBold(true);
    nameFont.setPointSizeF(nameFont.pointSizeF() * 1.25);
    nameLabel->setFont(nameFont);
    layout->addWidget(nameLabel);

    if (!title.isEmpty()) {
        auto *titleLabel = new QLabel(title, page);
        titleLabel->setObjectName(QStringLiteral("titleLabel"));
        titleLabel->setTextFormat(Qt::PlainText);
        titleLabel->setTextInteractionFlags(Qt::TextSelectableByMouse);
        titleLabel->setWordWrap(true);
        layout->addWidget(titleLabel);
    }
    layout->addSpacing(8);

    *outLayout = layout;
    return page;
}

} // namespace

QWidget *InspectorWidget::buildProductPage(const ProductDetailSnapshot &detail)
{
    QVBoxLayout *layout = nullptr;
    QWidget *page = makeContent(detail.name, detail.title, &layout);

    QFormLayout *general = startFormSection(layout, tr("General"), QStringLiteral("generalSection"));
    addFormRow(general, tr("Images"), QString::number(detail.imageCount));
    addFormRow(general, tr("Subsystems"), QString::number(detail.subsystemCount));
    addFormRow(general, tr("Entries"), QString::number(detail.entryCount));
    addFormRow(general, tr("Descriptor"), yesNo(detail.descriptorPresent));
    addFormRow(general, tr("IDB"), yesNo(detail.idbPresent));

    // Descriptor metadata only exists (and is only shown) when the
    // product actually has a descriptor.
    if (detail.descriptorPresent) {
        QFormLayout *descriptor =
            startFormSection(layout, tr("Descriptor"), QStringLiteral("descriptorSection"));
        addFormRow(descriptor, tr("Generation"), detail.descriptorGeneration);
        addFormRow(descriptor, tr("Layout"), QString::number(detail.descriptorLayoutLevel));
        addFormRow(descriptor, tr("Stamp"), QString::number(detail.descriptorStamp));
    }

    addHardwareSections(layout, detail.mach);

    if (!detail.cutpoints.isEmpty()) {
        QFormLayout *cutpoints =
            startFormSection(layout, tr("Cutpoints"), QStringLiteral("cutpointsSection"));
        for (const CutpointSnapshot &cutpoint : detail.cutpoints) {
            addFormRow(cutpoints, cutpoint.path, QString::number(cutpoint.sequence));
        }
    }

    layout->addStretch();
    return page;
}

QWidget *InspectorWidget::buildImagePage(const ImageDetailSnapshot &detail)
{
    QVBoxLayout *layout = nullptr;
    QWidget *page = makeContent(detail.name, detail.title, &layout);

    QFormLayout *general = startFormSection(layout, tr("General"), QStringLiteral("generalSection"));
    addFormRow(general, tr("Product"), detail.productName);
    // Unknown version/order show as "-", never as a fake 0.
    addFormRow(general,
               tr("Version"),
               detail.versionKnown ? QString::number(detail.version) : QStringLiteral("-"));
    addFormRow(general,
               tr("Order"),
               detail.orderKnown ? QString::number(detail.order) : QStringLiteral("-"));
    addFormRow(general, tr("Subsystems"), QString::number(detail.subsystemCount));
    addFormRow(general, tr("Entries"), QString::number(detail.entryCount));
    addFormRow(general, tr("Descriptor"), yesNo(detail.descriptorPresent));
    addFormRow(general, tr("IDB"), yesNo(detail.idbPresent));

    addHardwareSections(layout, detail.mach);

    layout->addStretch();
    return page;
}

QWidget *InspectorWidget::buildSubsystemPage(const SubsystemDetailSnapshot &detail)
{
    QVBoxLayout *layout = nullptr;
    QWidget *page = makeContent(detail.identity, detail.title, &layout);

    QFormLayout *general = startFormSection(layout, tr("General"), QStringLiteral("generalSection"));
    addFormRow(general, tr("Product"), detail.productName);
    addFormRow(general, tr("Image"), detail.imageName);
    addFormRow(general,
               tr("Version"),
               detail.imageVersionKnown ? QString::number(detail.imageVersion)
                                        : QStringLiteral("-"));
    addFormRow(general, tr("Entries"), QString::number(detail.entryCount));
    addFormRow(general, tr("Descriptor"), yesNo(detail.descriptorPresent));
    addFormRow(general, tr("IDB"), yesNo(detail.idbPresent));

    // IDB-only subsystem: descriptor-derived sections are hidden, and
    // a quiet note says why, instead of rendering unknown as empty.
    if (!detail.descriptorPresent) {
        auto *note = textLabel(tr("Descriptor information is unavailable."));
        note->setObjectName(QStringLiteral("descriptorUnavailableNote"));
        layout->addWidget(note);
    }

    addFlagsSection(layout, detail.flags);
    addHardwareSections(layout, detail.mach);

    if (detail.mappingKnown) {
        QVBoxLayout *mapping =
            startSection(layout, tr("Mapping"), QStringLiteral("mappingSection"));
        mapping->addWidget(
            textLabel(detail.mapping.isEmpty() ? QStringLiteral("-") : detail.mapping));
    }

    if (detail.rules.known) {
        addPrerequisitesSection(layout, detail.rules);
        addRangeSection(
            layout, tr("Replaces"), QStringLiteral("replacesSection"), detail.rules.replaces);
        addRangeSection(layout,
                        tr("Incompatibilities"),
                        QStringLiteral("incompatibilitiesSection"),
                        detail.rules.incompatibilities);
        addRangeSection(
            layout, tr("Updates"), QStringLiteral("updatesSection"), detail.rules.updates);
        addRangeSection(
            layout, tr("Follows"), QStringLiteral("followsSection"), detail.rules.follows);
    }

    if (detail.autominirootKnown) {
        addRangeSection(layout,
                        tr("Autominiroot"),
                        QStringLiteral("autominirootSection"),
                        detail.autominiroot);
    }

    layout->addStretch();
    return page;
}
