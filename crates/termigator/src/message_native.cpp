// See message_native.h.

#include "termigator/src/message_native.h"
#include "termigator/src/bell_native.h"

#if __has_include(<KNotification>)
#include <KNotification>
#define TERMIGATOR_HAVE_KNOTIFICATION 1
#else
#define TERMIGATOR_HAVE_KNOTIFICATION 0
#endif

#ifdef _WIN32
#define NOMINMAX
#define WIN32_LEAN_AND_MEAN
#include <windows.h> // MessageBeep, RegGetValueW

#include <string>
#endif

#include <QtCore/QLoggingCategory>
#include <QtGui/QGuiApplication>
#include <QtWidgets/QApplication>
#include <QtGui/QWindow>
#include <QtQuick/QQuickItem>
#include <QtQuick/QQuickWindow>
#include <QtWidgets/QAbstractButton>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QPushButton>

namespace termigator {

// QT_LOGGING_RULES="termigator.sound.debug=true": which sound was asked for and what played it.
Q_LOGGING_CATEGORY(lcSound, "termigator.sound", QtWarningMsg)

namespace {

QMessageBox::Icon iconFor(const QString &name)
{
    if (name == QLatin1String("information"))
        return QMessageBox::Information;
    if (name == QLatin1String("warning"))
        return QMessageBox::Warning;
    if (name == QLatin1String("critical"))
        return QMessageBox::Critical;
    if (name == QLatin1String("question"))
        return QMessageBox::Question;
    return QMessageBox::NoIcon;
}

// The button Esc / the window's close button stand for. QMessageBox detects it privately and only
// records it as the clicked button after `finished`, so this repeats its rule: the escape button
// set explicitly, else Cancel, else the only button, else the only Reject / No role button.
QAbstractButton *escapeButtonOf(const QMessageBox *box)
{
    if (QAbstractButton *b = box->escapeButton())
        return b;
    if (QAbstractButton *b = box->button(QMessageBox::Cancel))
        return b;
    const QList<QAbstractButton *> all = box->buttons();
    if (all.size() == 1)
        return all.first();
    QAbstractButton *found = nullptr;
    for (QAbstractButton *b : all) {
        const QMessageBox::ButtonRole role = box->buttonRole(b);
        if (role != QMessageBox::RejectRole && role != QMessageBox::NoRole)
            continue;
        if (found)
            return nullptr;
        found = b;
    }
    return found;
}

#ifdef _WIN32
// True when the Windows sound scheme gives the event `name` a sound file.
bool hasSystemSound(const wchar_t *name)
{
    std::wstring key = L"AppEvents\\Schemes\\Apps\\.Default\\";
    key += name;
    key += L"\\.Current";
    // Read the text itself: asked for the size only, RegGetValueW adds room for a terminator it
    // might append, so an empty value reports 4 bytes, not 2.
    wchar_t path[1024] = {};
    DWORD size = sizeof(path);
    return RegGetValueW(HKEY_CURRENT_USER, key.c_str(), nullptr, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                        nullptr, path, &size)
        == ERROR_SUCCESS
        && path[0] != L'\0';
}
#endif

// The sound KDE plays for its own message boxes (plasma-integration does it, and the AppImage ships
// no platform theme): the Plasma event of the icon, whose notifyrc action is just Sound|Taskbar.
void playSoundFor(const QString &icon)
{
    qCDebug(lcSound) << "message box sound for icon" << icon;
    const char *event = icon == QLatin1String("question")   ? "messageQuestion"
        : icon == QLatin1String("warning")                  ? "messageWarning"
        : icon == QLatin1String("critical")                 ? "messageCritical"
        : icon == QLatin1String("information")              ? "messageInformation"
                                                            : nullptr;
    if (!event)
        return;
#ifdef _WIN32
    // Windows' own sounds for these boxes (Control Panel -> Sound): no Plasma events there. The
    // default scheme gives "Question" no sound at all, so a question with none set plays the
    // "Exclamation" sound instead (these questions confirm closing or deleting something).
    const UINT type = icon == QLatin1String("question") ? (hasSystemSound(L"SystemQuestion") ? MB_ICONQUESTION : MB_ICONWARNING)
        : icon == QLatin1String("warning")              ? MB_ICONWARNING
        : icon == QLatin1String("critical")             ? MB_ICONERROR
                                                        : MB_ICONINFORMATION;
    const BOOL ok = MessageBeep(type);
    qCDebug(lcSound) << "message box" << icon << "MessageBeep" << Qt::hex << type << "->" << ok;
#elif TERMIGATOR_HAVE_KNOTIFICATION
    auto *n = new KNotification(QLatin1String(event));
    n->setComponentName(QStringLiteral("plasma_workspace"));
    n->sendEvent();
#else
    Q_UNUSED(event);
#endif
}

} // namespace

void playBell()
{
#ifdef _WIN32
    const BOOL ok = MessageBeep(MB_OK); // the "Default Beep" system sound
    qCDebug(lcSound) << "bell MessageBeep(MB_OK) ->" << ok;
#elif TERMIGATOR_HAVE_KNOTIFICATION
    // Plasma's "beep" event (bell-window-system), the sound KDE plays for a terminal bell.
    auto *n = new KNotification(QStringLiteral("beep"));
    n->setComponentName(QStringLiteral("plasma_workspace"));
    n->sendEvent();
#else
    QApplication::beep();
#endif
}

NativeMessageBox::NativeMessageBox(QObject *parent)
    : QObject(parent)
{
}

NativeMessageBox::~NativeMessageBox()
{
    dispose();
}

bool NativeMessageBox::isVisible() const
{
    return m_box && m_box->isVisible();
}

QString NativeMessageBox::dialogClass() const
{
    return m_box ? QString::fromLatin1(m_box->metaObject()->className()) : QString();
}

void NativeMessageBox::setTitle(const QString &v)
{
    if (m_title == v)
        return;
    m_title = v;
    Q_EMIT titleChanged();
}

void NativeMessageBox::setText(const QString &v)
{
    if (m_text == v)
        return;
    m_text = v;
    Q_EMIT textChanged();
}

void NativeMessageBox::setInformativeText(const QString &v)
{
    if (m_informativeText == v)
        return;
    m_informativeText = v;
    Q_EMIT informativeTextChanged();
}

void NativeMessageBox::setIcon(const QString &v)
{
    if (m_icon == v)
        return;
    m_icon = v;
    Q_EMIT iconChanged();
}

void NativeMessageBox::setButtons(int v)
{
    if (m_buttons == v)
        return;
    m_buttons = v;
    Q_EMIT buttonsChanged();
}

void NativeMessageBox::setButtonTexts(const QVariantMap &v)
{
    if (m_buttonTexts == v)
        return;
    m_buttonTexts = v;
    Q_EMIT buttonTextsChanged();
}

void NativeMessageBox::setDefaultButton(int v)
{
    if (m_defaultButton == v)
        return;
    m_defaultButton = v;
    Q_EMIT defaultButtonChanged();
}

void NativeMessageBox::setRichText(bool v)
{
    if (m_richText == v)
        return;
    m_richText = v;
    Q_EMIT richTextChanged();
}

void NativeMessageBox::setCheckBoxText(const QString &v)
{
    if (m_checkBoxText == v)
        return;
    m_checkBoxText = v;
    Q_EMIT checkBoxTextChanged();
}

void NativeMessageBox::setChecked(bool v)
{
    if (m_box && m_box->checkBox())
        m_box->checkBox()->setChecked(v);
    if (m_checked == v)
        return;
    m_checked = v;
    Q_EMIT checkedChanged();
}

// The window of the nearest QML ancestor (an Item's window or a Window), else the focused one.
QWindow *NativeMessageBox::hostWindow() const
{
    for (const QObject *o = parent(); o; o = o->parent()) {
        if (const auto *w = qobject_cast<const QWindow *>(o))
            return const_cast<QWindow *>(w);
        if (const auto *item = qobject_cast<const QQuickItem *>(o)) {
            if (QWindow *w = item->window())
                return w;
        }
    }
    return QGuiApplication::focusWindow();
}

void NativeMessageBox::open()
{
    if (m_box)
        return;
    // A new dialog every time, never a reused one (see MessageBox.qml).
    auto *box = new QMessageBox;
    m_box = box;
    box->setIcon(iconFor(m_icon));
    box->setWindowTitle(m_title);
    box->setText(m_text);
    box->setTextFormat(m_richText ? Qt::RichText : Qt::PlainText);
    if (!m_informativeText.isEmpty())
        box->setInformativeText(m_informativeText);
    // The button texts and their order come from the platform theme.
    box->setStandardButtons(QMessageBox::StandardButtons(m_buttons));
    for (auto it = m_buttonTexts.cbegin(); it != m_buttonTexts.cend(); ++it) {
        bool ok = false;
        const int which = it.key().toInt(&ok);
        if (QAbstractButton *b = ok ? box->button(QMessageBox::StandardButton(which)) : nullptr)
            b->setText(it.value().toString());
    }
    if (m_defaultButton != 0) {
        if (auto *b = qobject_cast<QPushButton *>(box->button(QMessageBox::StandardButton(m_defaultButton))))
            box->setDefaultButton(b);
    }
    if (!m_checkBoxText.isEmpty()) {
        auto *check = new QCheckBox(m_checkBoxText, box);
        check->setChecked(m_checked);
        box->setCheckBox(check);
    }
    connect(box, &QDialog::finished, this, [this, box] { finish(box); });

    QWindow *host = hostWindow();
    // The native window must exist to get a transient parent (Wayland places and stacks by it).
    box->winId();
    if (host && box->windowHandle()) {
        box->windowHandle()->setTransientParent(host);
        box->open(); // window modal: blocks only the host window, does not block the event loop
    } else {
        box->setWindowModality(Qt::ApplicationModal);
        box->show();
    }
    playSoundFor(m_icon);
    Q_EMIT visibleChanged();
}

void NativeMessageBox::finish(QMessageBox *box)
{
    if (box != m_box)
        return;
    QAbstractButton *clicked = box->clickedButton();
    if (!clicked)
        clicked = escapeButtonOf(box);
    int button = 0;
    int role = -1;
    if (clicked) {
        button = box->standardButton(clicked);
        role = box->buttonRole(clicked);
    }
    if (QCheckBox *check = box->checkBox(); check && check->isChecked() != m_checked) {
        m_checked = check->isChecked();
        Q_EMIT checkedChanged();
    }
    m_box.clear();
    box->deleteLater();
    if (clicked)
        Q_EMIT buttonClicked(button, role);
    Q_EMIT visibleChanged();
}

void NativeMessageBox::dispose()
{
    if (!m_box)
        return;
    QMessageBox *box = m_box;
    m_box.clear();
    box->disconnect(this);
    box->hide();
    box->deleteLater();
}

void NativeMessageBox::click(int standardButton)
{
    if (!m_box)
        return;
    if (QAbstractButton *b = m_box->button(QMessageBox::StandardButton(standardButton)))
        b->click();
}

void NativeMessageBox::dismiss()
{
    if (m_box)
        m_box->close();
}

QString NativeMessageBox::buttonText(int standardButton) const
{
    if (!m_box)
        return QString();
    QAbstractButton *b = m_box->button(QMessageBox::StandardButton(standardButton));
    return b ? b->text() : QString();
}

int NativeMessageBox::shownDefaultButton() const
{
    if (!m_box || !m_box->defaultButton())
        return 0;
    return m_box->standardButton(m_box->defaultButton());
}

void NativeMessageBox::close()
{
    if (!m_box)
        return;
    dispose();
    Q_EMIT visibleChanged();
}

} // namespace termigator
