#pragma once

// Box drawing (U+2500–U+257F) and block elements (U+2580–U+259F) drawn as shapes on the cell
// grid, as Konsole and ghostty do. A font's own glyphs for them rarely advance exactly one cell
// (many monospace fonts lack them and a fallback with other metrics steps in), so a long frame
// drawn from glyphs drifts off its columns and its corners miss the lines they join (k9s, mc).

#include <QtCore/QRectF>

class QColor;
class QPainter;

namespace termigator::term {

// True for the characters drawBoxChar() draws.
bool isBoxChar(char32_t c);

// Draws `c` filling `cell` in `color`. Returns false (and draws nothing) for a character it does
// not handle.
bool drawBoxChar(QPainter *painter, const QRectF &cell, char32_t c, const QColor &color);

} // namespace termigator::term
