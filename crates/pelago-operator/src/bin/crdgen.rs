use kube::CustomResourceExt;
use pelago_operator::api::PelagoSite;

fn main() {
    let crd = PelagoSite::crd();
    println!("{}", serde_yaml::to_string(&crd).expect("serialize CRD"));
}
