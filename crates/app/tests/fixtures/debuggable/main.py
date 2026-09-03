def compute():
    answer = 42
    return answer


def main():
    value = compute()
    print(f"E2E_DEBUG_MARKER {value}", flush=True)


main()
