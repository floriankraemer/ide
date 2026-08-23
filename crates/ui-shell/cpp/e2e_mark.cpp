#include "e2e_mark.h"

#include <QByteArray>

#include <cstdio>
#include <cstdlib>

namespace {

// Opened once. Re-opening per call would cost a syscall on paths the view
// takes thousands of times, and appending from two handles interleaves.
std::FILE *markStream()
{
    static std::FILE *const stream = []() -> std::FILE * {
        const char *path = std::getenv("IDE_E2E_EVENTS");
        if (path == nullptr || *path == '\0') {
            return nullptr;
        }
        return std::fopen(path, "ae");
    }();
    return stream;
}

} // namespace

void e2eMark(const char *json)
{
    std::FILE *stream = markStream();
    if (stream == nullptr) {
        return;
    }
    std::fputs(json, stream);
    std::fputc('\n', stream);
    // Flushed per line: a test reads this file while the app is still
    // running, and a crash must not swallow the marks that explain it.
    std::fflush(stream);
}

void e2eMark(const QString &json)
{
    if (markStream() == nullptr) {
        return;
    }
    e2eMark(json.toUtf8().constData());
}

QString e2eJson(const QString &value)
{
    QString out;
    out.reserve(value.size() + 2);
    out += QLatin1Char('"');
    for (const QChar character : value) {
        const char16_t code = character.unicode();
        switch (code) {
        case u'"':
            out += QLatin1String("\\\"");
            break;
        case u'\\':
            out += QLatin1String("\\\\");
            break;
        case u'\n':
            out += QLatin1String("\\n");
            break;
        case u'\r':
            out += QLatin1String("\\r");
            break;
        case u'\t':
            out += QLatin1String("\\t");
            break;
        default:
            if (code < 0x20) {
                out += QString::asprintf("\\u%04x", code);
            } else {
                out += character;
            }
            break;
        }
    }
    out += QLatin1Char('"');
    return out;
}
