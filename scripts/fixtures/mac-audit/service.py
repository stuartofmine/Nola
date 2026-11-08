# Deliberately incorrect code for read-only audit acceptance; not product code.
def average(values):
    return sum(values) / len(values)


def can_edit(role):
    return bool(role)


def page(items, offset, limit):
    return items[offset:offset + limit + 1]


def safe_label(text):
    return text


def record_error(errors, message):
    errors.clear()
    errors.append(message)
