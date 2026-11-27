// Box drawing and block elements on the cell grid. See box_drawing.h.

#include "termigator-term-bridge/src/box_drawing.h"

#include <QtGui/QColor>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QPen>

namespace termigator::term {

namespace {

// The four arms of U+2500–U+257F, up, right, down, left: 0 none, 1 light, 2 heavy, 3 double;
// "...." for the dashed and diagonal lines, which keep their glyphs.
constexpr char LINE_ARMS[] =
    "0101020210102020................................0110021001200220"
    "0011001200210022110012002100220010011002200120021110121021101120"
    "2120221012202220101110122011102120212012102220220111011202110212"
    "0121012202210222110111021201120221012102220122021111111212111212"
    "211111212121211222111122122122121222212222212222................"
    "0303303003100130033000130031003313003100330010033001300313103130"
    "3330101330313033031301310333130331013303131331313333............"
    "................000110000100001000022000020000200201102001022010";

struct Arms
{
    int up = 0;
    int right = 0;
    int down = 0;
    int left = 0;
};

bool lineArms(char32_t c, Arms &arms)
{
    if (c < 0x2500 || c > 0x257f)
        return false;
    const char *entry = LINE_ARMS + (c - 0x2500) * 4;
    if (entry[0] == '.')
        return false;
    arms = Arms{entry[0] - '0', entry[1] - '0', entry[2] - '0', entry[3] - '0'};
    return true;
}

// The rounded corners ╭╮╯╰ (U+256D–U+2570).
bool isArc(char32_t c)
{
    return c >= 0x256d && c <= 0x2570;
}

void drawLines(QPainter *painter, const QRectF &cell, const Arms &arms, const QColor &color,
               qreal light)
{
    const qreal heavy = light * 2;
    const auto thickness = [&](int weight) { return weight == 2 ? heavy : light; };
    const qreal cx = cell.left() + cell.width() / 2;
    const qreal cy = cell.top() + cell.height() / 2;
    // The two strokes of a double line sit this far either side of the centre line.
    const qreal gap = light;
    const qreal half = light / 2;
    // How far an arm reaches past the centre so it covers the strokes across it.
    const auto reach = [&](int a, int b) {
        qreal r = 0;
        for (const int weight : {a, b}) {
            if (weight == 3)
                r = qMax(r, gap + half);
            else if (weight != 0)
                r = qMax(r, thickness(weight) / 2);
        }
        return r;
    };
    const qreal acrossVertical = reach(arms.up, arms.down);
    const qreal acrossHorizontal = reach(arms.left, arms.right);
    const auto hbar = [&](qreal x1, qreal x2, qreal y, qreal t) {
        painter->fillRect(QRectF(x1, y - t / 2, x2 - x1, t), color);
    };
    const auto vbar = [&](qreal y1, qreal y2, qreal x, qreal t) {
        painter->fillRect(QRectF(x - t / 2, y1, t, y2 - y1), color);
    };

    if (arms.left == 1 || arms.left == 2)
        hbar(cell.left(), cx + acrossVertical, cy, thickness(arms.left));
    if (arms.right == 1 || arms.right == 2)
        hbar(cx - acrossVertical, cell.right(), cy, thickness(arms.right));
    if (arms.up == 1 || arms.up == 2)
        vbar(cell.top(), cy + acrossHorizontal, cx, thickness(arms.up));
    if (arms.down == 1 || arms.down == 2)
        vbar(cy - acrossHorizontal, cell.bottom(), cx, thickness(arms.down));

    // A double arm's outer stroke turns the corner; its inner stroke stops at the arm across it.
    if (arms.left == 3) {
        hbar(cell.left(), cx + (arms.up != 0 ? -gap : gap) + half, cy - gap, light);
        hbar(cell.left(), cx + (arms.down != 0 ? -gap : gap) + half, cy + gap, light);
    }
    if (arms.right == 3) {
        hbar(cx + (arms.up != 0 ? gap : -gap) - half, cell.right(), cy - gap, light);
        hbar(cx + (arms.down != 0 ? gap : -gap) - half, cell.right(), cy + gap, light);
    }
    if (arms.up == 3) {
        vbar(cell.top(), cy + (arms.left != 0 ? -gap : gap) + half, cx - gap, light);
        vbar(cell.top(), cy + (arms.right != 0 ? -gap : gap) + half, cx + gap, light);
    }
    if (arms.down == 3) {
        vbar(cy + (arms.left != 0 ? gap : -gap) - half, cell.bottom(), cx - gap, light);
        vbar(cy + (arms.right != 0 ? gap : -gap) - half, cell.bottom(), cx + gap, light);
    }
}

void drawArc(QPainter *painter, const QRectF &cell, char32_t c, const QColor &color, qreal light)
{
    const qreal cx = cell.left() + cell.width() / 2;
    const qreal cy = cell.top() + cell.height() / 2;
    // ╭ joins down and right, ╮ down and left, ╯ up and left, ╰ up and right.
    const qreal vy = (c == 0x256d || c == 0x256e) ? cell.bottom() : cell.top();
    const qreal hx = (c == 0x256d || c == 0x2570) ? cell.right() : cell.left();
    QPainterPath path(QPointF(cx, vy));
    path.quadTo(QPointF(cx, cy), QPointF(hx, cy));
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, true);
    QPen pen(color, light);
    pen.setCapStyle(Qt::FlatCap);
    painter->setPen(pen);
    painter->setBrush(Qt::NoBrush);
    painter->drawPath(path);
    painter->restore();
}

// U+2580–U+259F.
bool drawBlock(QPainter *painter, const QRectF &cell, char32_t c, const QColor &color)
{
    const qreal x = cell.left();
    const qreal y = cell.top();
    const qreal w = cell.width();
    const qreal h = cell.height();
    if (c == 0x2580) { // ▀
        painter->fillRect(QRectF(x, y, w, h / 2), color);
    } else if (c >= 0x2581 && c <= 0x2588) { // ▁ … █, lower eighths
        const qreal part = h * (c - 0x2580) / 8;
        painter->fillRect(QRectF(x, y + h - part, w, part), color);
    } else if (c >= 0x2589 && c <= 0x258f) { // ▉ … ▏, left eighths
        painter->fillRect(QRectF(x, y, w * (0x2590 - c) / 8, h), color);
    } else if (c == 0x2590) { // ▐
        painter->fillRect(QRectF(x + w / 2, y, w / 2, h), color);
    } else if (c >= 0x2591 && c <= 0x2593) { // ░ ▒ ▓
        QColor shade = color;
        shade.setAlphaF(color.alphaF() * (c - 0x2590) / 4);
        painter->fillRect(cell, shade);
    } else if (c == 0x2594) { // ▔
        painter->fillRect(QRectF(x, y, w, h / 8), color);
    } else if (c == 0x2595) { // ▕
        painter->fillRect(QRectF(x + w * 7 / 8, y, w / 8, h), color);
    } else if (c >= 0x2596 && c <= 0x259f) { // quadrants: 1 upper left, 2 upper right, 4 lower left, 8 lower right
        static constexpr int QUADRANTS[] = {4, 8, 1, 13, 9, 7, 11, 2, 6, 14};
        const int q = QUADRANTS[c - 0x2596];
        if (q & 1)
            painter->fillRect(QRectF(x, y, w / 2, h / 2), color);
        if (q & 2)
            painter->fillRect(QRectF(x + w / 2, y, w / 2, h / 2), color);
        if (q & 4)
            painter->fillRect(QRectF(x, y + h / 2, w / 2, h / 2), color);
        if (q & 8)
            painter->fillRect(QRectF(x + w / 2, y + h / 2, w / 2, h / 2), color);
    } else {
        return false;
    }
    return true;
}

} // namespace

bool isBoxChar(char32_t c)
{
    Arms arms;
    return lineArms(c, arms) || isArc(c) || (c >= 0x2580 && c <= 0x259f);
}

bool drawBoxChar(QPainter *painter, const QRectF &cell, char32_t c, const QColor &color)
{
    // A light stroke is an eighth of the cell width, at least a pixel (Konsole's proportion).
    const qreal light = qMax<qreal>(1, qRound(cell.width() / 8));
    Arms arms;
    if (lineArms(c, arms)) {
        drawLines(painter, cell, arms, color, light);
        return true;
    }
    if (isArc(c)) {
        drawArc(painter, cell, c, color, light);
        return true;
    }
    return drawBlock(painter, cell, c, color);
}

} // namespace termigator::term
