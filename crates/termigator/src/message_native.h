#pragma once

// The system message box for qml/shell/MessageBox.qml: a real QMessageBox (QtWidgets).
// QtQuick.Dialogs MessageDialog only gets a native look when the platform theme provides a
// message dialog helper; plasma-integration does not (it falls back to a QML dialog), so the
// look would depend on the Plasma version. GUI thread only, after the QApplication exists.

#include <QtCore/QObject>
#include <QtCore/QPointer>
#include <QtCore/QString>
#include <QtQml/qqmlregistration.h>

class QMessageBox;
class QWindow;

namespace termigator {

class NativeMessageBox : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(QString title READ title WRITE setTitle NOTIFY titleChanged)
    Q_PROPERTY(QString text READ text WRITE setText NOTIFY textChanged)
    // "none", "information", "warning", "critical" or "question".
    Q_PROPERTY(QString icon READ icon WRITE setIcon NOTIFY iconChanged)
    // QMessageBox::StandardButtons; the same values as the QtQuick.Dialogs MessageDialog buttons.
    Q_PROPERTY(int buttons READ buttons WRITE setButtons NOTIFY buttonsChanged)
    // true: Qt::RichText (e.g. Help → About); default false keeps setTextFormat(PlainText).
    Q_PROPERTY(bool richText READ richText WRITE setRichText NOTIFY richTextChanged)
    // Non-empty: a check box with this text under the message (QMessageBox::setCheckBox); `checked`
    // is its state, read back when the dialog closes.
    Q_PROPERTY(QString checkBoxText READ checkBoxText WRITE setCheckBoxText NOTIFY checkBoxTextChanged)
    Q_PROPERTY(bool checked READ isChecked WRITE setChecked NOTIFY checkedChanged)
    Q_PROPERTY(bool visible READ isVisible NOTIFY visibleChanged)
    // Always true: tells this dialog from the scene (QQC2) fallback in tests.
    Q_PROPERTY(bool native READ isNative CONSTANT)
    // Meta-object class name of the dialog shown now ("QMessageBox"), else empty.
    Q_PROPERTY(QString dialogClass READ dialogClass NOTIFY visibleChanged)

public:
    explicit NativeMessageBox(QObject *parent = nullptr);
    ~NativeMessageBox() override;

    QString title() const { return m_title; }
    QString text() const { return m_text; }
    QString icon() const { return m_icon; }
    int buttons() const { return m_buttons; }
    bool richText() const { return m_richText; }
    QString checkBoxText() const { return m_checkBoxText; }
    bool isChecked() const { return m_checked; }
    bool isVisible() const;
    bool isNative() const { return true; }
    QString dialogClass() const;

    void setTitle(const QString &v);
    void setText(const QString &v);
    void setIcon(const QString &v);
    void setButtons(int v);
    void setRichText(bool v);
    void setCheckBoxText(const QString &v);
    // Also ticks the check box of the dialog shown now (a test hook as well).
    void setChecked(bool v);

    // Shows a fresh QMessageBox, window modal over the window this object lives in, without
    // blocking. Does nothing while one is shown.
    Q_INVOKABLE void open();
    // Hides the dialog without a buttonClicked signal.
    Q_INVOKABLE void close();
    // Test hooks: click a standard button; close the window like its title bar button (Esc does the
    // same as clicking the escape button).
    Q_INVOKABLE void click(int standardButton);
    Q_INVOKABLE void dismiss();

Q_SIGNALS:
    // Once per dialog: the clicked button, or the escape button for Esc / the window's close button.
    void buttonClicked(int button, int role);
    void titleChanged();
    void textChanged();
    void iconChanged();
    void buttonsChanged();
    void richTextChanged();
    void checkBoxTextChanged();
    void checkedChanged();
    void visibleChanged();

private:
    QWindow *hostWindow() const;
    void finish(QMessageBox *box);
    void dispose();

    QString m_title = QStringLiteral("Termigator");
    QString m_text;
    QString m_icon = QStringLiteral("information");
    int m_buttons = 0x00000400; // Ok
    bool m_richText = false;
    QString m_checkBoxText;
    bool m_checked = false;
    QPointer<QMessageBox> m_box;
};

} // namespace termigator
