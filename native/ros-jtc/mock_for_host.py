#!/usr/bin/env python3
"""ROS-only simulator for Rust Host integration; never loads any ROS hardware plugin."""
import argparse,json,os,time,threading,uuid
from pathlib import Path
import rclpy
from rclpy.action import ActionServer,GoalResponse
from rclpy.callback_groups import ReentrantCallbackGroup
from rclpy.executors import MultiThreadedExecutor
from control_msgs.action import FollowJointTrajectory
from controller_manager_msgs.srv import ListControllers
from controller_manager_msgs.msg import ControllerState
p=argparse.ArgumentParser();p.add_argument('--state',type=Path,required=True);a=p.parse_args()
os.environ['RMW_IMPLEMENTATION']='rmw_fastrtps_cpp';os.environ['ROS_AUTOMATIC_DISCOVERY_RANGE']='LOCALHOST';os.environ['ROS_LOG_DIR']='/tmp/ros-jtc-host-mock'
state={'controller_session':str(uuid.uuid4()),'goals':[],'received':0,'test_only':True};lock=threading.Lock()
def save():
    temp=a.state.with_suffix('.tmp');temp.write_text(json.dumps(state));temp.replace(a.state)
rclpy.init(domain_id=171);node=rclpy.create_node('rx_test_jtc_host_server');group=ReentrantCallbackGroup()
def controllers(_req,res):
    c=ControllerState();c.name='arm_controller';c.type='joint_trajectory_controller/JointTrajectoryController';c.state='active';c.claimed_interfaces=['joint'+str(i)+'/position' for i in range(1,7)];res.controller=[c];return res
def accept(_req):
    with lock:state['received']+=1;save()
    return GoalResponse.ACCEPT
def execute(handle):
    with lock:state['goals'].append(str(uuid.UUID(bytes=bytes(handle.goal_id.uuid))));save()
    time.sleep(.2);handle.succeed();return FollowJointTrajectory.Result()
node.create_service(ListControllers,'/robot/controller_manager/list_controllers',controllers,callback_group=group)
server=ActionServer(node,FollowJointTrajectory,'/robot/arm_controller/follow_joint_trajectory',goal_callback=accept,execute_callback=execute,callback_group=group)
executor=MultiThreadedExecutor(num_threads=4);executor.add_node(node);save()
try:executor.spin()
finally:executor.shutdown();node.destroy_node();rclpy.shutdown()
