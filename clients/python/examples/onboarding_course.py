from __future__ import annotations

import os

from pelagodb import PelagoClient


def main() -> None:
    endpoint = os.getenv("PELAGO_ENDPOINT", "127.0.0.1:27615")
    database = os.getenv("PELAGO_DATABASE", "default")
    namespace = os.getenv("PELAGO_NAMESPACE", "onboarding.demo")

    client = PelagoClient(endpoint, database=database, namespace=namespace)

    person_schema = {
        "name": "Person",
        "properties": {
            "name": {"type": "string", "required": True},
            "role": {"type": "string", "index": "equality"},
            "level": {"type": "int", "index": "range"},
        },
    }
    client.register_schema_dict(person_schema)

    node = client.create_node(
        "Person",
        {
            "name": "Py Onboard",
            "role": "Analyst",
            "level": 3,
        },
    )
    updated = client.update_node("Person", node.id, {"level": 4})

    print(f"python created node id={node.id}")
    print(f"python updated node id={updated.id} level=4")

    print("python find level >= 3")
    for row in client.find_nodes("Person", "level >= 3", limit=10):
        print(f"- {row.id} {row.entity_type}")

    print("python pql level >= 3")
    for result in client.execute_pql("Person @filter(level >= 3) { uid name role level }"):
        if result.node:
            print(f"- {result.node.id}")

    client.close()


if __name__ == "__main__":
    main()
