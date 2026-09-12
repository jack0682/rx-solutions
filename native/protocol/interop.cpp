#include "strict_wire.hpp"
#include "rx/contract/v1/contract.pb.h"
#include "rx/cell/v1/cell.pb.h"
#include <google/protobuf/dynamic_message.h>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <sstream>
#include <stdexcept>

std::string read(const std::filesystem::path& path) {
  std::ifstream file(path, std::ios::binary);
  if (!file) throw std::runtime_error("missing fixture: "+path.string());
  return {std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>()};
}
int main(int argc,char** argv) {
  try {
    if(argc!=2) throw std::runtime_error("fixture directory required");
    const std::filesystem::path folder(argv[1]);
    // These references retain both generated descriptor registrations in static builds.
    (void)rx::contract::v1::Intent::descriptor();
    (void)rx::cell::v1::StartRunRequest::descriptor();
    google::protobuf::DynamicMessageFactory factory;
    std::ifstream index(folder/"cases.tsv");
    if(!index) throw std::runtime_error("missing cases.tsv");
    std::size_t tested=0;
    std::string line;
    while(std::getline(index,line)) {
      std::istringstream row(line);
      std::string name,type,expected;
      std::getline(row,name,'\t');std::getline(row,type,'\t');std::getline(row,expected,'\t');
      const auto* descriptor=google::protobuf::DescriptorPool::generated_pool()->FindMessageTypeByName(type);
      if(!descriptor) throw std::runtime_error("unknown fixture message: "+type);
      const auto bytes=read(folder/(name+".pb"));
      std::string error;
      const std::unique_ptr<google::protobuf::Message> message(factory.GetPrototype(descriptor)->New());
      const bool accepted=rx::wire::parse(*message,bytes,error);
      if(accepted!=(expected=="valid")) throw std::runtime_error("fixture mismatch: "+name+" "+error);
      if(accepted) {
        std::ofstream output(folder/(name+".return.pb"),std::ios::binary);
        if(!message->SerializeToOstream(&output)) throw std::runtime_error("roundtrip serialization failed");
      }
      ++tested;
    }
    rx::contract::v1::Intent intent;
    if(!intent.ParseFromString(read(folder/"cv01.pb"))
       || intent.kind()!=rx::contract::v1::KIND_ENSURE_STATE
       || intent.target()!="example/fixture"
       || intent.body().predicate().target().boolean()!=true
       || intent.execution_timeout_ms()!=5000
       || intent.profile_digest()!=std::string(32,'\0')) throw std::runtime_error("independent CV01 field assertions failed");
    if(tested<17) throw std::runtime_error("fixture coverage incomplete");
    std::cout<<"C++ Protobuf "<<GOOGLE_PROTOBUF_VERSION<<": "<<tested<<" wire fixtures passed\n";
    return 0;
  } catch(const std::exception& e) { std::cerr<<e.what()<<"\n"; return 1; }
}
