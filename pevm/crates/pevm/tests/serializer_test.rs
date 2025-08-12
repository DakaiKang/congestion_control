use pevm::serialization::serializer;

#[test]
fn test_serializer() {
    serializer::try_one_serialization();
}


#[test]
fn test_adapter_and_serializer() {
    serializer::try_one_adapt();
}