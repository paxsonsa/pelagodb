"""End-to-end example: typed schema, namespaces, cross-namespace edges, and query builder.

Requires a running PelagoDB server at localhost:27615.
"""

from pelagodb import (
    PelagoClient, Namespace, Entity, Property, OutEdge, IndexType
)


# --- Schema definitions ---

class GlobalNamespace(Namespace):
    name = "global"

    class Vendor(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        industry: str = Property()


class TenantNamespace(Namespace):
    name = "tenant_{tenant_id}"

    class Person(Entity):
        name: str = Property(required=True, index=IndexType.EQUALITY)
        age: int = Property(default=0, index=IndexType.RANGE)
        active: bool = Property(default=True)
        follows = OutEdge("Person")
        supplied_by = OutEdge(GlobalNamespace.Vendor)


def main() -> None:
    # --- Connect and register ---
    client = PelagoClient("localhost:27615")
    client.register(GlobalNamespace)

    acme = TenantNamespace.bind(tenant_id="acme")
    client.register(acme)

    # --- Scoped operations ---
    global_ns = client.ns(GlobalNamespace)
    vendor = global_ns.create(GlobalNamespace.Vendor(name="Acme Corp", industry="Tech"))
    print(f"Created vendor: {vendor}")

    acme_ns = client.ns(acme)
    alice = acme_ns.create(acme.Person(name="Alice", age=31))
    bob = acme_ns.create(acme.Person(name="Bob", age=29))
    charlie = acme_ns.create(acme.Person(name="Charlie", age=25))
    print(f"Created: {alice}, {bob}, {charlie}")

    # --- Cross-namespace link ---
    client.link(alice, "supplied_by", vendor)
    client.link(alice, "follows", bob)
    client.link(alice, "follows", charlie)
    client.link(bob, "follows", alice)

    # --- Point lookup ---
    fetched = acme_ns.get(acme.Person, alice.id)
    print(f"Fetched Alice: {fetched}")

    # --- Update ---
    updated = acme_ns.update(alice, age=32)
    print(f"Updated Alice age: {updated.age}")

    # --- Filter scan ---
    print("\nPeople over 30:")
    for p in acme_ns.find(acme.Person, acme.Person.age > 30):
        print(f"  {p.name} (age={p.age})")

    # --- Query builder ---
    print("\nAlice's follows (age > 25):")
    results = (
        acme_ns.query(acme.Person)
        .filter(acme.Person.name == "Alice")
        .traverse(acme.Person.follows, filter=acme.Person.age > 25)
        .limit(20)
        .run()
    )
    for p in results:
        print(f"  {p.name}")

    # --- Unlink and delete ---
    client.unlink(alice, "follows", bob)
    acme_ns.delete(charlie)

    client.close()
    print("\nDone.")


if __name__ == "__main__":
    main()
