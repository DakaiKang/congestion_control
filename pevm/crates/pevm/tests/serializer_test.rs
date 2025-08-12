use pevm::serialization::serializer;

// #[test]
// fn test_serializer() {
//     serializer::test();
// }

#[tokio::test]
async fn test_serializer() -> Result<(), Box<dyn std::error::Error>> {
    serializer::test().await
}