from pelagodb import PelagoClient


def main() -> None:
    client = PelagoClient("127.0.0.1:27615", database="default", namespace="demo")

    schema = {
        "name": "Person",
        "properties": {
            "name": {"type": "string", "required": True},
            "age": {"type": "int", "index": "range"},
        },
        "edges": {
            "follows": {"target": "Person", "direction": "outgoing"},
        },
    }
    client.register_schema_dict(schema)

    alice = client.create_node("Person", {"name": "Alice", "age": 31})
    bob = client.create_node("Person", {"name": "Bob", "age": 29})
    client.create_edge("Person", alice.id, "follows", "Person", bob.id)

    fetched = client.get_node("Person", alice.id)
    print("Alice:", fetched.id if fetched else "missing")

    for row in client.find_nodes("Person", "age >= 30", limit=20):
        print(row.id, row.entity_type, row.locality)

    client.close()


if __name__ == "__main__":
    main()
