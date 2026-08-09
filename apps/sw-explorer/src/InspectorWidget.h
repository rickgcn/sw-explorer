#pragma once

#include <QWidget>

#include "EntrySnapshot.h"
#include "InspectorSnapshot.h"

QT_BEGIN_NAMESPACE
class QLabel;
class QStackedWidget;
QT_END_NAMESPACE

// The right-hand detail pane: shows the object details behind the
// current tree selection.
//
// The widget is a QStackedWidget with Empty / Loading / Error pages
// plus one scrollable content page per object kind. Content pages are
// rebuilt from the incoming snapshot on every show*() call, so a
// section that does not apply simply does not exist on the page (and
// tests can assert its absence via its objectName).
//
// Everything here is plain Qt Widgets in the native style: QLabel,
// QGroupBox, QFormLayout, QVBoxLayout, QScrollArea.
class InspectorWidget : public QWidget
{
    Q_OBJECT

public:
    explicit InspectorWidget(QWidget *parent = nullptr);

    void showEmpty();
    void showLoading();
    void showProduct(const ProductDetailSnapshot &detail);
    void showImage(const ImageDetailSnapshot &detail);
    void showSubsystem(const SubsystemDetailSnapshot &detail);
    void showEntry(const EntryDetailSnapshot &detail);
    void showError(const QString &message);

private:
    QWidget *buildProductPage(const ProductDetailSnapshot &detail);
    QWidget *buildImagePage(const ImageDetailSnapshot &detail);
    QWidget *buildSubsystemPage(const SubsystemDetailSnapshot &detail);
    QWidget *buildEntryPage(const EntryDetailSnapshot &detail);
    void showContent(const QString &pageName, QWidget *content);

    QStackedWidget *m_stack = nullptr;
    QWidget *m_emptyPage = nullptr;
    QWidget *m_loadingPage = nullptr;
    QWidget *m_errorPage = nullptr;
    QLabel *m_errorMessage = nullptr;
    // Single slot for the current content page; the widget inside is
    // replaced on every show*() call.
    QWidget *m_contentPage = nullptr;
};
