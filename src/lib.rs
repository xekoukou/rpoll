extern crate libc;

/*The data types that are to be used by the extern functions. */

use libc::{c_int,uint32_t};

#[repr(C)]
struct epoll_data_t {
fd: c_int,
// Is this necessary?
dummy: c_int
}

#[repr(C)]
struct epoll_event_t {
    events: uint32_t,
    data: epoll_data_t
}
extern {

fn epoll_create1(flags: c_int) -> c_int;
fn epoll_ctl(epfd:c_int,op:c_int,fd:c_int,epoll_event:*mut epoll_event_t) ->c_int;
fn epoll_wait(epfd: c_int,events:*mut epoll_event_t,maxevents: c_int, timeout: c_int) ->c_int;
fn close(fd:c_int) -> c_int;
}

use std::net::TcpStream;
use std::net::TcpListener;
use std::io::Error;
use std::os::unix::io::AsRawFd;



#[cfg(target_os = "linux")]
struct Poll<'a> {
    fd: c_int,
    streams: Vec<(c_int,TcpStream,&'a mut FnMut(&mut Poll,&TcpStream)->c_int)>,
    listeners: Vec<(c_int,TcpListener,&'a mut FnMut(&TcpListener)->c_int)>,
    timers: Vec<(c_int,c_int)>,
    current_time_wall: c_int,
    current_time: c_int
}

enum Poll_Event {
    EPOLLIN = 0x001,
    EPOLLOUT = 0x004
}


#[cfg(target_os = "linux")]
impl<'a> Poll<'a> {

    fn new() -> Result<Self,Error> {
        let fd;
        unsafe {
            fd = epoll_create1(0);
        }
        if(fd<0) {
            Err(std::io::Error::last_os_error())
        } else {
        Ok(Poll {fd:fd,streams:Vec::new(),listeners:Vec::new(),timers:Vec::new(),current_time:0})
        }
    }

    fn attach_stream(&mut self,stream:TcpStream,event:Poll_Event,callback: &'a mut FnMut(&mut Poll,&TcpStream)->i32) -> Result<Result<i32,Error>,()> {
       let fd = stream.as_raw_fd();
       match self.streams.binary_search_by(|a| a.0.cmp(&fd)) {
           Ok(_) => Err(()),
           Err(_) => {
       self.streams.push((fd,stream,callback));
       let no_error;
       unsafe {
           let mut epoll_event = epoll_event_t {events:event as uint32_t,data:epoll_data_t {fd:fd, dummy:0}};
           no_error = epoll_ctl(self.fd,1,fd, &mut epoll_event);
       }
       if(no_error<0) {
           Ok(Err(std::io::Error::last_os_error()))
       } else {
           self.streams.sort_by(|a,b| a.0.cmp(&b.0));
           Ok(Ok(fd))
       }
       }
       } 
    }

    fn detach_stream(&mut self,id:i32) -> Result<Result<TcpStream,Error>,()> {
        match self.streams.binary_search_by(|a| a.0.cmp(&id)) {     
            Ok(index) => {
                let (fd,stream,callback) = self.streams.remove(index);
                let no_error;
                unsafe {
                    let mut epoll_event = epoll_event_t {events:0,data:epoll_data_t {fd:0, dummy:0}};
                    no_error = epoll_ctl(self.fd,2,fd,&mut epoll_event);
                }
                if(no_error<0) {
                    self.streams.push((fd,stream,callback));
                    Ok(Err(std::io::Error::last_os_error()))
                } else {
                    Ok(Ok(stream))
                }
            },
           Err(_) => {
               Err(())
           }            
        }
    }

    fn wait(&mut self,timeout:i32) ->Result<(),Error> {
        let mut events:[epoll_event_t;200];
        let mut number_of_events;
        unsafe {
            events = std::mem::uninitialized();
            number_of_events = epoll_wait(self.fd,&mut events[0] as *mut epoll_event_t,200,timeout);
        }
        if number_of_events < 0 {
            Err(std::io::Error::last_os_error())
        } else {
        for i in 0..number_of_events {
            let fd = events[i as usize].data.fd;
            
            let index =  self.streams.binary_search_by(|a| a.0.cmp(&fd)).unwrap();
            let (_,stream,callback) = self.streams[index];
            let new_timeout = callback(self,&stream);
            self.timers.push((self.current_time_wall +new_timeout,fd));
        }
        self.wait(callback(self,&stream))
        }
    }

}

impl<'a> Drop for Poll<'a> {

    fn drop(&mut self) {
        loop {
            match self.streams.pop() {
                None => break,
                Some(s) => { 
                    match self.detach_stream(s.0) {
                        Ok(_) => (),
                        Err(_) => {
                            println!("Error: Poll Object could't be destroyed correctly");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        loop {
            match self.listeners.pop() {
                None => break,
                Some(s) => () 
            }
        }
        unsafe {
            close(self.fd);
        }
    }
}
