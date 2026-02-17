from pelagodb import PelagoClient


def main() -> None:
    client = PelagoClient("127.0.0.1:27615", api_key="dev-admin-key")

    # Example: inspect audit history.
    audit = client.query_audit(limit=20)
    for event in audit.events:
        print(event.timestamp, event.action, event.allowed)

    client.close()


if __name__ == "__main__":
    main()
