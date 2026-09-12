#include <onnxruntime_cxx_api.h>
#include <iostream>
int main(int argc,char**argv) {
  if(argc!=2) return 2;
  try {
    Ort::Env env(ORT_LOGGING_LEVEL_WARNING,"rx-model-check");
    Ort::SessionOptions options;options.SetIntraOpNumThreads(1);options.SetInterOpNumThreads(1);
    Ort::Session session(env,argv[1],options);
    const auto inputs=session.GetInputCount(),outputs=session.GetOutputCount();
    if(inputs==0||outputs==0)return 3;
    std::cout<<"{\"inputs\":"<<inputs<<",\"outputs\":"<<outputs<<"}\n";
  } catch(const Ort::Exception&error) {std::cerr<<error.what()<<'\n';return 1;}
}
