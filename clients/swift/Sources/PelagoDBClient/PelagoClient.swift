import Foundation
import GRPC
import NIO

public final class PelagoClient {
    private let group: EventLoopGroup
    private let channel: GRPCChannel
    private let database: String
    private let namespace: String
    private let siteID: String

    public init(
        host: String,
        port: Int,
        database: String = "default",
        namespace: String = "default",
        siteID: String = ""
    ) {
        self.group = MultiThreadedEventLoopGroup(numberOfThreads: 1)
        self.channel = ClientConnection.insecure(group: group)
            .connect(host: host, port: port)
        self.database = database
        self.namespace = namespace
        self.siteID = siteID
    }

    deinit {
        try? channel.close().wait()
        try? group.syncShutdownGracefully()
    }

    // Extend with generated stub calls after running Scripts/generate_proto.sh.
    // Suggested wrappers:
    // - registerSchema
    // - createNode
    // - getNode
    // - findNodes
    // - executePQL
}
