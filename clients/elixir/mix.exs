defmodule PelagoDB.MixProject do
  use Mix.Project

  def project do
    [
      app: :pelagodb_client,
      version: "0.1.0",
      elixir: "~> 1.15",
      start_permanent: Mix.env() == :prod,
      deps: deps()
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp deps do
    [
      {:grpc, "~> 0.8"},
      {:protobuf, "~> 0.12"},
      {:jason, "~> 1.4"}
    ]
  end
end
