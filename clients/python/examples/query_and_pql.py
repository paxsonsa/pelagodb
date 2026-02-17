from pelagodb import PelagoClient


def main() -> None:
    client = PelagoClient("127.0.0.1:27615", database="default", namespace="demo")

    for node in client.find_nodes("Person", "age >= 30", limit=100):
        print("find:", node.id)

    for result in client.execute_pql("Person @filter(age >= 30) { uid name age }"):
        if result.explain:
            print("explain:", result.explain)
        if result.node:
            print("pql node:", result.node.id)

    client.close()


if __name__ == "__main__":
    main()
